//! SubSpace "core" transport over UDP: encryption handshake, reliable ordered delivery with
//! acks and retransmit, small/huge chunk reassembly, packet clusters and time sync.
//! Everything that is not a core packet (first byte != 0) is surfaced as a game packet.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{SystemTime, UNIX_EPOCH};

use super::vie::Vie;
use super::continuum::{ContinuumCipher, MAX_CIPHERTEXT};

pub const MAX_PACKET: usize = 520;
const RELIABLE_HEADER: usize = 6;
const RESEND_TICKS: u32 = 100;
const ENCRYPT_RESEND_TICKS: u32 = 100;
const ENCRYPT_MAX_ATTEMPTS: u32 = 10;
const SYNC_INTERVAL_TICKS: u32 = 500;
const TIMEOUT_TICKS: u32 = 1500;

/// Local time in 1/100 s ticks, 31-bit like the protocol's timestamps.
pub fn now_tick() -> u32 {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    ((ms / 10) as u32) & 0x7FFF_FFFF
}

fn tick_diff(a: u32, b: u32) -> i32 {
    (a.wrapping_sub(b) << 1) as i32 >> 1
}

fn u32_at(p: &[u8], o: usize) -> Option<u32> {
    p.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Encryption handshake completed; the session can log in.
    Connected,
    /// A game (non-core) packet, delivered in order for reliable traffic.
    Packet(Vec<u8>),
    Disconnected(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Handshake { attempts: u32, last_sent: u32 },
    Connected,
    Disconnected,
}

struct Outgoing {
    id: u32,
    payload: Vec<u8>,
    sent_tick: u32,
}

struct ContinuumSession {
    keys: Option<(u32, u32)>,
    cipher: Option<ContinuumCipher>,
    expand: Box<dyn FnMut(u32) -> Option<[u32; 20]> + Send>,
}

pub struct Transport {
    sock: UdpSocket,
    vie: Vie,
    continuum: Option<ContinuumSession>,
    pub state: State,
    next_reliable_id: u32,
    unacked: Vec<Outgoing>,
    next_expected_id: u32,
    pending_in: BTreeMap<u32, Vec<u8>>,
    acks_out: Vec<u32>,
    small_chunk: Vec<u8>,
    huge_chunk: Vec<u8>,
    events: VecDeque<Event>,
    last_recv_tick: u32,
    last_sync_tick: u32,
    /// Server tick minus local tick, from sync exchanges.
    pub time_diff: i32,
    /// Round-trip time in ticks from the latest sync.
    pub rtt: u32,
    pub packets_sent: u32,
    pub packets_received: u32,
}

impl Transport {
    pub fn connect(server: SocketAddr) -> io::Result<Self> {
        Self::connect_inner(server, None)
    }

    /// Negotiates Continuum transport using a caller-supplied expanded-key lookup.
    /// The callback receives key2 and returns its PRE-finalization 20-word table.
    /// It must return promptly; None explicitly fails negotiation. No key expansion
    /// implementation or service is bundled. This does not implement Continuum login.
    pub fn connect_continuum(
        server: SocketAddr,
        expand: impl FnMut(u32) -> Option<[u32; 20]> + Send + 'static,
    ) -> io::Result<Self> {
        Self::connect_inner(server, Some(ContinuumSession {
            keys: None,
            cipher: None,
            expand: Box::new(expand),
        }))
    }

    fn connect_inner(server: SocketAddr, continuum: Option<ContinuumSession>) -> io::Result<Self> {
        let bind: SocketAddr = if server.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }.parse().unwrap();
        let sock = UdpSocket::bind(bind)?;
        sock.connect(server)?;
        sock.set_nonblocking(true)?;
        let now = now_tick();
        let mut t = Self {
            sock,
            vie: Vie::new(now ^ std::process::id().rotate_left(16)),
            continuum,
            state: State::Handshake { attempts: 0, last_sent: now },
            next_reliable_id: 0,
            unacked: Vec::new(),
            next_expected_id: 0,
            pending_in: BTreeMap::new(),
            acks_out: Vec::new(),
            small_chunk: Vec::new(),
            huge_chunk: Vec::new(),
            events: VecDeque::new(),
            last_recv_tick: now,
            last_sync_tick: now,
            time_diff: 0,
            rtt: 0,
            packets_sent: 0,
            packets_received: 0,
        };
        t.send_encryption_request(now);
        Ok(t)
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.sock.local_addr()
    }

    pub fn server_tick(&self) -> u32 {
        (now_tick() as i64 + self.time_diff as i64) as u32 & 0x7FFF_FFFF
    }

    fn send_encryption_request(&mut self, now: u32) {
        let mut p = Vec::with_capacity(8);
        if let Some((key1, _)) = self.continuum.as_ref().and_then(|c| c.keys) {
            p.extend_from_slice(&[0x00, 0x11]);
            p.extend_from_slice(&key1.to_le_bytes());
            p.extend_from_slice(&1u16.to_le_bytes());
        } else {
            p.extend_from_slice(&[0x00, 0x01]);
            p.extend_from_slice(&self.vie.client_key.to_le_bytes());
            let protocol = if self.continuum.is_some() { 0x11u16 } else { 1u16 };
            p.extend_from_slice(&protocol.to_le_bytes());
        }
        // These negotiation packets remain plaintext even after a table is installed.
        self.send_wire(&p);
        if let State::Handshake { attempts, .. } = self.state {
            self.state = State::Handshake { attempts: attempts + 1, last_sent: now };
        }
    }

    fn send_raw(&mut self, data: &[u8]) {
        let buf = if let Some(cont) = &self.continuum {
            let Some(cipher) = &cont.cipher else { return };
            let Ok(buf) = cipher.encrypt(data) else { return };
            buf
        } else {
            let mut buf = data.to_vec();
            self.vie.encrypt(&mut buf);
            buf
        };
        self.send_wire(&buf);
    }

    fn send_wire(&mut self, buf: &[u8]) {
        // A full socket buffer just drops the datagram, as UDP would.
        let _ = self.sock.send(buf);
        self.packets_sent = self.packets_sent.wrapping_add(1);
    }

    fn accept_continuum_response(&mut self, packet: &[u8]) {
        if packet.len() != 12 {
            return;
        }
        let Some(cont) = self.continuum.as_mut() else { return };
        // Once a table is selected, retransmit our existing acknowledgement without
        // re-expanding or accepting a stale/reordered challenge as a fresh session.
        if cont.keys.is_some() {
            return;
        }
        if packet[10..12] != [1, 0] {
            self.fail("unsupported Continuum encryption revision");
            return;
        }
        let key1 = u32_at(packet, 2).unwrap();
        let key2 = u32_at(packet, 6).unwrap();
        let Some(table) = (cont.expand)(key2) else {
            self.fail("Continuum key expansion is unavailable; no session table was supplied");
            return;
        };
        cont.cipher = Some(ContinuumCipher::from_expanded_table(table, key1));
        cont.keys = Some((key1, key2));
        let now = now_tick();
        self.packets_received = self.packets_received.wrapping_add(1);
        self.last_recv_tick = now;
        self.state = State::Handshake { attempts: 0, last_sent: now };
        self.send_encryption_request(now);
    }

    /// Sends an unreliable packet.
    pub fn send(&mut self, data: &[u8]) {
        debug_assert!(data.len() <= MAX_PACKET);
        self.send_raw(data);
    }

    /// Sends a packet with guaranteed, ordered delivery.
    pub fn send_reliable(&mut self, payload: &[u8]) {
        assert!(payload.len() + RELIABLE_HEADER <= MAX_PACKET, "reliable payload too large");
        let id = self.next_reliable_id;
        self.next_reliable_id = self.next_reliable_id.wrapping_add(1);
        let out = Outgoing { id, payload: payload.to_vec(), sent_tick: now_tick() };
        self.send_reliable_frame(&out);
        self.unacked.push(out);
    }

    fn send_reliable_frame(&mut self, out: &Outgoing) {
        let mut p = Vec::with_capacity(RELIABLE_HEADER + out.payload.len());
        p.extend_from_slice(&[0x00, 0x03]);
        p.extend_from_slice(&out.id.to_le_bytes());
        p.extend_from_slice(&out.payload);
        self.send_raw(&p);
    }

    pub fn send_sync_request(&mut self) {
        let mut p = vec![0x00, 0x05];
        p.extend_from_slice(&now_tick().to_le_bytes());
        p.extend_from_slice(&self.packets_sent.to_le_bytes());
        p.extend_from_slice(&self.packets_received.to_le_bytes());
        self.send_raw(&p);
        self.last_sync_tick = now_tick();
    }

    pub fn disconnect(&mut self) {
        if self.state != State::Disconnected {
            self.send_raw(&[0x00, 0x07]);
            self.state = State::Disconnected;
        }
    }

    /// Receives and processes everything pending, handles retransmits and acks, and returns
    /// the resulting events.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut buf = [0u8; 2048];
        loop {
            match self.sock.recv(&mut buf) {
                Ok(n) if n > 0 => {
                    // Continuum adds a CRC and possibly an escape byte. Keep the
                    // receive buffer larger than either limit to detect truncation.
                    let limit = if self.continuum.is_some() { MAX_CIPHERTEXT } else { MAX_PACKET };
                    if n > limit || self.state == State::Disconnected {
                        continue;
                    }
                    let pkt = if let Some(cont) = &self.continuum {
                        if matches!(self.state, State::Handshake { .. }) && buf[..n].starts_with(&[0, 0x10]) {
                            self.accept_continuum_response(&buf[..n]);
                            continue;
                        }
                        let Some(cipher) = &cont.cipher else { continue };
                        let Ok(pkt) = cipher.decrypt(&buf[..n]) else { continue };
                        pkt
                    } else {
                        let mut pkt = buf[..n].to_vec();
                        self.vie.decrypt(&mut pkt);
                        pkt
                    };
                    // Do not deliver game packets or process reliable/chunk state
                    // until the server has acknowledged the encryption session.
                    if matches!(self.state, State::Handshake { .. })
                        && !pkt.starts_with(&[0, 2]) && pkt != [0, 7] {
                        continue;
                    }
                    self.packets_received = self.packets_received.wrapping_add(1);
                    self.last_recv_tick = now_tick();
                    self.handle(&pkt);
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
                    // ICMP port unreachable from an earlier send; keep trying during handshake.
                    if self.state == State::Connected {
                        self.fail("server unreachable");
                    }
                    break;
                }
                Err(e) => {
                    self.fail(&format!("socket error: {e}"));
                    break;
                }
            }
        }
        self.maintain();
        self.events.drain(..).collect()
    }

    fn fail(&mut self, why: &str) {
        if self.state != State::Disconnected {
            self.state = State::Disconnected;
            self.events.push_back(Event::Disconnected(why.to_string()));
        }
    }

    fn maintain(&mut self) {
        let now = now_tick();
        match self.state {
            State::Handshake { attempts, last_sent } => {
                if tick_diff(now, last_sent) >= ENCRYPT_RESEND_TICKS as i32 {
                    if attempts >= ENCRYPT_MAX_ATTEMPTS {
                        let waiting_for_ack = self.continuum.as_ref().is_some_and(|c| c.keys.is_some());
                        self.fail(if waiting_for_ack {
                            "Continuum encryption acknowledgement timed out"
                        } else {
                            "no response from server"
                        });
                    } else {
                        self.send_encryption_request(now);
                    }
                }
            }
            State::Connected => {
                let due: Vec<usize> = (0..self.unacked.len())
                    .filter(|&i| tick_diff(now, self.unacked[i].sent_tick) >= RESEND_TICKS as i32)
                    .collect();
                for i in due {
                    self.unacked[i].sent_tick = now;
                    let frame = Outgoing { id: self.unacked[i].id, payload: self.unacked[i].payload.clone(), sent_tick: now };
                    self.send_reliable_frame(&frame);
                }
                if tick_diff(now, self.last_sync_tick) >= SYNC_INTERVAL_TICKS as i32 {
                    self.send_sync_request();
                }
                if tick_diff(now, self.last_recv_tick) >= TIMEOUT_TICKS as i32 {
                    self.fail("connection timed out");
                }
            }
            State::Disconnected => {}
        }
        self.flush_acks();
    }

    fn flush_acks(&mut self) {
        let acks = std::mem::take(&mut self.acks_out);
        // One cluster holds up to (520 - 2) / 7 acks.
        for group in acks.chunks((MAX_PACKET - 2) / 7) {
            if group.len() == 1 {
                let mut p = vec![0x00, 0x04];
                p.extend_from_slice(&group[0].to_le_bytes());
                self.send_raw(&p);
            } else {
                let mut p = vec![0x00, 0x0E];
                for id in group {
                    p.extend_from_slice(&[6, 0x00, 0x04]);
                    p.extend_from_slice(&id.to_le_bytes());
                }
                self.send_raw(&p);
            }
        }
    }

    fn handle(&mut self, pkt: &[u8]) {
        if pkt.is_empty() {
            return;
        }
        if pkt[0] != 0x00 {
            self.events.push_back(Event::Packet(pkt.to_vec()));
            return;
        }
        let Some(&kind) = pkt.get(1) else { return };
        match kind {
            0x12 if self.state == State::Connected && pkt.len() == 6 => {
                let seed = u32_at(pkt, 2).unwrap();
                if let Some(cont) = self.continuum.as_mut() {
                    if let Some(table) = (cont.expand)(seed) {
                        let mut response = vec![0, 0x13];
                        response.extend_from_slice(&seed.to_le_bytes());
                        for word in table {
                            response.extend_from_slice(&word.to_le_bytes());
                        }
                        self.send_raw(&response);
                    } else {
                        self.fail("Continuum key expansion challenge could not be answered");
                    }
                }
            }
            0x02 => {
                if let (State::Handshake { .. }, Some(key)) = (self.state, u32_at(pkt, 2)) {
                    let valid = if let Some(cont) = &self.continuum {
                        cont.cipher.is_some() && key == 0
                    } else {
                        self.vie.initialize(key)
                    };
                    if valid {
                        self.state = State::Connected;
                        self.events.push_back(Event::Connected);
                        self.send_sync_request();
                    } else {
                        self.fail(if self.continuum.is_some() {
                            "server sent an invalid Continuum encryption acknowledgement"
                        } else {
                            "server sent an invalid encryption key"
                        });
                    }
                }
            }
            0x03 => {
                let Some(id) = u32_at(pkt, 2) else { return };
                self.acks_out.push(id);
                if id < self.next_expected_id || self.pending_in.contains_key(&id) {
                    return;
                }
                self.pending_in.insert(id, pkt[RELIABLE_HEADER..].to_vec());
                while let Some(payload) = self.pending_in.remove(&self.next_expected_id) {
                    self.next_expected_id = self.next_expected_id.wrapping_add(1);
                    self.handle(&payload);
                }
            }
            0x04 => {
                if let Some(id) = u32_at(pkt, 2) {
                    self.unacked.retain(|o| o.id != id);
                }
            }
            0x05 => {
                if let Some(their) = u32_at(pkt, 2) {
                    let mut p = vec![0x00, 0x06];
                    p.extend_from_slice(&their.to_le_bytes());
                    p.extend_from_slice(&now_tick().to_le_bytes());
                    self.send_raw(&p);
                }
            }
            0x06 => {
                if let (Some(sent), Some(server)) = (u32_at(pkt, 2), u32_at(pkt, 6)) {
                    let now = now_tick();
                    let rtt = tick_diff(now, sent).max(0);
                    self.rtt = rtt as u32;
                    let diff = (rtt * 3) / 5 + tick_diff(server, now);
                    self.time_diff = if (-10..=10).contains(&diff) { 0 } else { diff };
                }
            }
            0x07 => self.fail("server closed the connection"),
            0x08 => self.small_chunk.extend_from_slice(&pkt[2..]),
            0x09 => {
                self.small_chunk.extend_from_slice(&pkt[2..]);
                let whole = std::mem::take(&mut self.small_chunk);
                self.handle(&whole);
            }
            0x0A => {
                let Some(total) = u32_at(pkt, 2) else { return };
                self.huge_chunk.extend_from_slice(&pkt[6..]);
                if self.huge_chunk.len() >= total as usize {
                    let whole = std::mem::take(&mut self.huge_chunk);
                    self.handle(&whole);
                }
            }
            0x0B => {
                self.huge_chunk.clear();
                self.send_raw(&[0x00, 0x0C]);
            }
            0x0E => {
                let mut i = 2;
                while i < pkt.len() {
                    let len = pkt[i] as usize;
                    let Some(sub) = pkt.get(i + 1..i + 1 + len) else { break };
                    let sub = sub.to_vec();
                    self.handle(&sub);
                    i += 1 + len;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake server socket paired with a transport, without encryption (server echoes the key).
    fn pair() -> (Transport, UdpSocket, SocketAddr) {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server.set_read_timeout(Some(std::time::Duration::from_millis(500))).unwrap();
        let mut t = Transport::connect(server.local_addr().unwrap()).unwrap();
        let mut buf = [0u8; 64];
        let (n, client) = server.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..2], &[0x00, 0x01]);
        assert_eq!(n, 8);
        let mut resp = vec![0x00, 0x02];
        resp.extend_from_slice(&buf[2..6]);
        server.send_to(&resp, client).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(t.poll().contains(&Event::Connected));
        drain(&server);
        (t, server, client)
    }

    fn drain(s: &UdpSocket) -> Vec<Vec<u8>> {
        // Let in-flight loopback datagrams arrive.
        std::thread::sleep(std::time::Duration::from_millis(30));
        s.set_nonblocking(true).unwrap();
        let mut out = Vec::new();
        let mut buf = [0u8; 2048];
        while let Ok((n, _)) = s.recv_from(&mut buf) {
            out.push(buf[..n].to_vec());
        }
        s.set_nonblocking(false).unwrap();
        out
    }

    fn reliable(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![0x00, 0x03];
        p.extend_from_slice(&id.to_le_bytes());
        p.extend_from_slice(payload);
        p
    }

    fn settle(t: &mut Transport) -> Vec<Event> {
        std::thread::sleep(std::time::Duration::from_millis(20));
        t.poll()
    }

    #[test]
    fn oversized_encrypted_datagrams_are_dropped() {
        let (mut t, server, client) = pair();
        assert!(t.vie.initialize(t.vie.client_key.wrapping_neg()));
        for len in [2048, MAX_PACKET + 1, 4096] {
            server.send_to(&vec![0x03; len], client).unwrap();
            assert!(settle(&mut t).is_empty());
        }
        let mut valid = vec![0x03; MAX_PACKET];
        let expected = valid.clone();
        t.vie.encrypt(&mut valid);
        server.send_to(&valid, client).unwrap();
        assert_eq!(settle(&mut t), vec![Event::Packet(expected)]);
        assert_eq!(t.state, State::Connected);
    }

    #[test]
    fn reliable_messages_are_ordered_deduplicated_and_acked() {
        let (mut t, server, client) = pair();
        server.send_to(&reliable(1, &[0x07, 2]), client).unwrap();
        server.send_to(&reliable(0, &[0x07, 1]), client).unwrap();
        server.send_to(&reliable(0, &[0x07, 1]), client).unwrap();
        let ev = settle(&mut t);
        assert_eq!(ev, vec![Event::Packet(vec![0x07, 1]), Event::Packet(vec![0x07, 2])]);
        let acks = drain(&server);
        let acked: usize = acks
            .iter()
            .map(|p| match p[1] {
                0x04 => 1,
                0x0E => (p.len() - 2) / 7,
                _ => 0,
            })
            .sum();
        assert_eq!(acked, 3, "every reliable copy is acked, including duplicates");
    }

    #[test]
    fn clusters_and_chunks_reassemble() {
        let (mut t, server, client) = pair();
        let cluster = [0x00, 0x0E, 2, 0x28, 1, 3, 0x29, 2, 3];
        server.send_to(&cluster, client).unwrap();
        server.send_to(&[0x00, 0x08, 0x2A, b'a'], client).unwrap();
        server.send_to(&[0x00, 0x09, b'b', b'c'], client).unwrap();
        let mut huge = vec![0x00, 0x0A];
        huge.extend_from_slice(&5u32.to_le_bytes());
        server.send_to(&[huge.clone(), vec![0x0F, 1, 2]].concat(), client).unwrap();
        server.send_to(&[huge, vec![3, 4]].concat(), client).unwrap();
        let ev = settle(&mut t);
        assert_eq!(
            ev,
            vec![
                Event::Packet(vec![0x28, 1]),
                Event::Packet(vec![0x29, 2, 3]),
                Event::Packet(vec![0x2A, b'a', b'b', b'c']),
                Event::Packet(vec![0x0F, 1, 2, 3, 4]),
            ]
        );
    }

    #[test]
    fn unacked_reliable_is_resent_and_ack_stops_it() {
        let (mut t, server, _client) = pair();
        let reliables = |s: &UdpSocket| -> Vec<Vec<u8>> { drain(s).into_iter().filter(|p| p[1] == 0x03).collect() };
        t.send_reliable(&[0x09, 1, 2]);
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(reliables(&server), vec![reliable(0, &[0x09, 1, 2])]);
        // Age the message past the resend delay.
        t.unacked[0].sent_tick = now_tick().wrapping_sub(RESEND_TICKS + 1);
        t.poll();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(reliables(&server), vec![reliable(0, &[0x09, 1, 2])]);
        let mut ack = vec![0x00, 0x04];
        ack.extend_from_slice(&0u32.to_le_bytes());
        server.send_to(&ack, t.local_addr().unwrap()).unwrap();
        settle(&mut t);
        assert!(t.unacked.is_empty());
    }

    #[test]
    fn answers_server_sync_request() {
        let (mut t, server, client) = pair();
        let mut req = vec![0x00, 0x05];
        req.extend_from_slice(&1234u32.to_le_bytes());
        server.send_to(&req, client).unwrap();
        settle(&mut t);
        let out = drain(&server);
        let resp = out.iter().find(|p| p[1] == 0x06).expect("sync response");
        assert_eq!(u32_at(resp, 2), Some(1234));
    }
}
