//! Synthetic loopback tests. These do not derive keys or log into a real zone.
use aicontinuum::net::continuum::ContinuumCipher;
use aicontinuum::net::transport::{Event, State, Transport, now_tick};
use std::net::{SocketAddr, UdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

const RESPONSE: [u8; 12] = [
    0, 0x10, 0x60, 0xc0, 0x6a, 0x8e, 0xb1, 0x0c, 0xcd, 0x83, 1, 0,
];
const ACK: [u8; 8] = [0, 0x11, 0x60, 0xc0, 0x6a, 0x8e, 1, 0];

fn pair(
    expand: impl FnMut(u32) -> Option<[u32; 20]> + Send + 'static,
) -> (Transport, UdpSocket, SocketAddr) {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    server
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let client = Transport::connect_continuum(server.local_addr().unwrap(), expand).unwrap();
    let mut buf = [0; 64];
    let (n, addr) = server.recv_from(&mut buf).unwrap();
    assert_eq!(n, 8);
    assert_eq!(&buf[..2], &[0, 1]);
    assert_eq!(&buf[6..8], &[0x11, 0]);
    (client, server, addr)
}

fn settle(client: &mut Transport) -> Vec<Event> {
    std::thread::sleep(Duration::from_millis(20));
    client.poll()
}

fn receive(server: &UdpSocket) -> Vec<u8> {
    server.set_nonblocking(false).unwrap();
    let mut buf = [0; 2048];
    let n = server.recv(&mut buf).unwrap();
    buf[..n].to_vec()
}

fn no_packet(server: &UdpSocket) {
    server.set_nonblocking(true).unwrap();
    let result = server.recv(&mut [0; 2048]);
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    server.set_nonblocking(false).unwrap();
}

fn reference_cipher() -> ContinuumCipher {
    // C++ vector: key1=0x8e6ac060 and a synthetic all-zero expanded table.
    // Using the precomputed FINAL table avoids making the simulated server
    // depend on the client's Rust finalization implementation.
    let line = include_str!("../docs/research/continuum/finalization-vectors.txt")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .nth(2)
        .unwrap();
    let fields: Vec<_> = line.split_whitespace().collect();
    assert_eq!(fields[0], "8e6ac060");
    let key = std::array::from_fn(|i| {
        let bytes = std::array::from_fn(|j| {
            let at = i * 8 + j * 2;
            u8::from_str_radix(&fields[2][at..at + 2], 16).unwrap()
        });
        u32::from_le_bytes(bytes)
    });
    ContinuumCipher::from_final_session_table(key)
}

#[test]
fn negotiates_and_transports_encrypted_reliable_packets() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let (mut client, server, addr) = pair(move |seed| {
        assert_eq!(seed, 0x83cd0cb1);
        count.fetch_add(1, Ordering::SeqCst);
        Some([0; 20])
    });
    let cipher = reference_cipher();
    server.send_to(&RESPONSE, addr).unwrap();
    assert!(settle(&mut client).is_empty());
    assert_eq!(receive(&server), ACK);
    assert!(matches!(client.state, State::Handshake { .. }));

    // Duplicate/stale challenges do not repeatedly expand or replace the table.
    server.send_to(&RESPONSE, addr).unwrap();
    let mut stale = RESPONSE;
    stale[2] ^= 1;
    server.send_to(&stale, addr).unwrap();
    assert!(settle(&mut client).is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    no_packet(&server);

    // Lost server acknowledgement: repeat 00 11 in plaintext, never 00 01.
    client.state = State::Handshake {
        attempts: 1,
        last_sent: now_tick().wrapping_sub(101),
    };
    assert!(client.poll().is_empty());
    assert_eq!(receive(&server), ACK);

    for early in [&[0x03, 7][..], &[0][..], &[0, 3, 0, 0, 0, 0, 9][..]] {
        server
            .send_to(&cipher.encrypt(early).unwrap(), addr)
            .unwrap();
        assert!(settle(&mut client).is_empty());
    }
    no_packet(&server);
    server.send_to(&[0, 2, 0, 0, 0, 0, 0], addr).unwrap();
    assert!(
        settle(&mut client).is_empty(),
        "plaintext cannot complete encrypted negotiation"
    );
    server
        .send_to(&cipher.encrypt(&[0, 2, 0, 0, 0, 0, 0]).unwrap(), addr)
        .unwrap();
    assert_eq!(settle(&mut client), vec![Event::Connected]);
    assert_eq!(client.state, State::Connected);
    assert_eq!(&cipher.decrypt(&receive(&server)).unwrap()[..2], &[0, 5]);

    // A duplicate encrypted acknowledgement must not trigger login again.
    server
        .send_to(&cipher.encrypt(&[0, 2, 0, 0, 0, 0, 0]).unwrap(), addr)
        .unwrap();
    assert!(settle(&mut client).is_empty());
    no_packet(&server);
    client.send_reliable(&[9, 1, 2]);
    assert_eq!(
        cipher.decrypt(&receive(&server)).unwrap(),
        [0, 3, 0, 0, 0, 0, 9, 1, 2]
    );
    for packet in [[0, 3, 1, 0, 0, 0, 8, 2], [0, 3, 0, 0, 0, 0, 8, 1]] {
        server
            .send_to(&cipher.encrypt(&packet).unwrap(), addr)
            .unwrap();
    }
    assert_eq!(
        settle(&mut client),
        vec![Event::Packet(vec![8, 1]), Event::Packet(vec![8, 2])]
    );
    assert_eq!(&cipher.decrypt(&receive(&server)).unwrap()[..2], &[0, 0x0e]);

    // The negotiated wire budget must allow 520 plaintext + CRC + escape.
    let (plain, wire) = (0u32..10000)
        .find_map(|value| {
            let mut plain = vec![3; 520];
            plain[1..5].copy_from_slice(&value.to_le_bytes());
            let wire = cipher.encrypt(&plain).unwrap();
            (wire.len() == 522).then_some((plain, wire))
        })
        .expect("escaped maximum-length fixture");
    server.send_to(&vec![0x03; 523], addr).unwrap();
    assert!(settle(&mut client).is_empty());
    server.send_to(&wire, addr).unwrap();
    assert_eq!(settle(&mut client), vec![Event::Packet(plain)]);
}

#[test]
fn missing_expansion_fails_without_acknowledging_or_sending_login() {
    let (mut client, server, addr) = pair(|_| None);
    server.send_to(&RESPONSE, addr).unwrap();
    assert_eq!(
        settle(&mut client),
        vec![Event::Disconnected(
            "Continuum key expansion is unavailable; no session table was supplied".into()
        )]
    );
    assert_eq!(client.state, State::Disconnected);
    no_packet(&server);
    server.send_to(&RESPONSE, addr).unwrap();
    assert!(settle(&mut client).is_empty());
    no_packet(&server);
}

#[test]
fn malformed_responses_cannot_invoke_the_provider() {
    let (mut client, server, addr) = pair(|_| panic!("unexpected expansion"));
    for n in 2..12 {
        server.send_to(&RESPONSE[..n], addr).unwrap();
    }
    let mut extra = RESPONSE.to_vec();
    extra.push(0);
    server.send_to(&extra, addr).unwrap();
    assert!(settle(&mut client).is_empty());
    no_packet(&server);
    let mut unsupported = RESPONSE;
    unsupported[10] = 2;
    server.send_to(&unsupported, addr).unwrap();
    assert_eq!(
        settle(&mut client),
        vec![Event::Disconnected(
            "unsupported Continuum encryption revision".into()
        )]
    );
    no_packet(&server);
}

#[test]
fn acknowledgement_retries_are_bounded() {
    let (mut client, server, addr) = pair(|_| Some([0; 20]));
    server.send_to(&RESPONSE, addr).unwrap();
    assert!(settle(&mut client).is_empty());
    assert_eq!(receive(&server), ACK);
    client.state = State::Handshake {
        attempts: 10,
        last_sent: now_tick().wrapping_sub(101),
    };
    assert!(matches!(client.poll().as_slice(), [Event::Disconnected(_)]));
    no_packet(&server);
}

#[test]
fn wrong_expansion_cannot_complete_the_handshake() {
    let (mut client, server, addr) = pair(|_| Some([1; 20]));
    server.send_to(&RESPONSE, addr).unwrap();
    assert!(settle(&mut client).is_empty());
    assert_eq!(receive(&server), ACK);
    server
        .send_to(
            &reference_cipher().encrypt(&[0, 2, 0, 0, 0, 0, 0]).unwrap(),
            addr,
        )
        .unwrap();
    assert!(settle(&mut client).is_empty());
    assert!(matches!(client.state, State::Handshake { .. }));
    no_packet(&server);
    client.state = State::Handshake {
        attempts: 10,
        last_sent: now_tick().wrapping_sub(101),
    };
    assert_eq!(
        client.poll(),
        vec![Event::Disconnected(
            "Continuum encryption acknowledgement timed out".into()
        )]
    );
}

#[test]
fn accepts_continuum_acknowledgements_without_a_vie_key_requirement() {
    for ack in [vec![0, 2], vec![0, 2, 1, 0, 0, 0, 0]] {
        let (mut client, server, addr) = pair(|_| Some([0; 20]));
        server.send_to(&RESPONSE, addr).unwrap();
        assert!(settle(&mut client).is_empty());
        assert_eq!(receive(&server), ACK);
        server
            .send_to(&reference_cipher().encrypt(&ack).unwrap(), addr)
            .unwrap();
        assert_eq!(settle(&mut client), vec![Event::Connected]);
        assert_eq!(
            &reference_cipher().decrypt(&receive(&server)).unwrap()[..2],
            &[0, 5]
        );
    }
}

#[test]
fn answers_encrypted_expansion_challenge_without_changing_session_key() {
    let (mut client, server, addr) = pair(|seed| {
        if seed == 0x83cd0cb1 {
            Some([0; 20])
        } else if seed == 123 {
            Some([0xabcdef01; 20])
        } else {
            None
        }
    });
    let cipher = reference_cipher();
    server.send_to(&RESPONSE, addr).unwrap();
    settle(&mut client);
    assert_eq!(receive(&server), ACK);
    server
        .send_to(&cipher.encrypt(&[0, 2, 0, 0, 0, 0]).unwrap(), addr)
        .unwrap();
    assert_eq!(settle(&mut client), vec![Event::Connected]);
    receive(&server); // sync
    server
        .send_to(&cipher.encrypt(&[0, 0x12, 123, 0, 0, 0]).unwrap(), addr)
        .unwrap();
    assert!(settle(&mut client).is_empty());
    let response = cipher.decrypt(&receive(&server)).unwrap();
    assert_eq!(&response[..8], &[0, 0x13, 0, 0, 123, 0, 0, 0]);
    assert_eq!(&response[8..], &[1, 0xef, 0xcd, 0xab].repeat(20));
    // The public reference reads the seed and permits trailing bytes. Exercise
    // direct and clustered padded challenges, as well as the minimum form above.
    for request in [
        vec![0, 0x12, 123, 0, 0, 0, 0xa5, 0x5a],
        vec![0, 0x0e, 13, 0, 0x12, 123, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7],
    ] {
        server.send_to(&cipher.encrypt(&request).unwrap(), addr).unwrap();
        assert!(settle(&mut client).is_empty());
        let response = cipher.decrypt(&receive(&server)).unwrap();
        assert_eq!(&response[..8], &[0, 0x13, 0, 0, 123, 0, 0, 0]);
        assert_eq!(&response[8..], &[1, 0xef, 0xcd, 0xab].repeat(20));
    }
    server
        .send_to(&cipher.encrypt(&[0, 0x12, 123]).unwrap(), addr)
        .unwrap();
    assert!(settle(&mut client).is_empty());
    no_packet(&server);
    server
        .send_to(&cipher.encrypt(&[0, 0x12, 124, 0, 0, 0]).unwrap(), addr)
        .unwrap();
    assert!(matches!(
        settle(&mut client).as_slice(),
        [Event::Disconnected(_)]
    ));
}

#[test]
fn session_uses_continuum_login_layout_only_after_encrypted_ack() {
    use aicontinuum::net::session::{Config, Session};
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    server
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let config = Config {
        server: server.local_addr().unwrap(),
        name: "SyntheticCont".into(),
        password: "synthetic".into(),
        arena: None,
        map_dir: std::env::temp_dir(),
    };
    let mut session = Session::connect_continuum(config, |_| Some([0; 20]), [0x5a; 64]).unwrap();
    let mut buf = [0; 2048];
    let (_, addr) = server.recv_from(&mut buf).unwrap();
    server.send_to(&RESPONSE, addr).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    session.update();
    assert_eq!(receive(&server), ACK);
    no_packet(&server);
    let cipher = reference_cipher();
    server
        .send_to(&cipher.encrypt(&[0, 2, 0, 0, 0, 0]).unwrap(), addr)
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    session.update();
    assert_eq!(&cipher.decrypt(&receive(&server)).unwrap()[..2], &[0, 5]);
    let reliable = cipher.decrypt(&receive(&server)).unwrap();
    assert_eq!(&reliable[..2], &[0, 3]);
    let login = &reliable[6..];
    assert_eq!(login.len(), 165);
    assert_eq!(&login[..2], &[0x24, 0]);
    assert_eq!(&login[2..15], b"SyntheticCont");
    assert_eq!(&login[75..77], &40u16.to_le_bytes());
    // Original 0.40 and SS.NET LoginPacket: IPv4 numeric value in LE order,
    // followed by the client's actual bound port, login ID, and reserved word.
    assert_eq!(&login[89..93], &[1, 0, 0, 127]);
    assert_eq!(&login[93..95], &addr.port().to_le_bytes());
    assert_eq!(&login[95..101], &[0; 6]);
    assert_eq!(&login[101..], &[0x5a; 64]);
    session.disconnect();
}

#[test]
fn completes_handshake_with_independent_legacy_crc_short_ack() {
    let line = include_str!("../docs/research/continuum/legacy-crc-vectors.txt")
        .lines()
        .last()
        .unwrap();
    let fields: Vec<_> = line.split_whitespace().collect();
    assert_eq!(fields[1], "0002");
    let wire: Vec<_> = fields[2]
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect();
    assert!(reference_cipher().decrypt(&wire).is_err());
    let (mut client, server, addr) = pair(|_| Some([0; 20]));
    server.send_to(&RESPONSE, addr).unwrap();
    assert!(settle(&mut client).is_empty());
    assert_eq!(receive(&server), ACK);
    server.send_to(&wire, addr).unwrap();
    assert_eq!(settle(&mut client), vec![Event::Connected]);
    assert_eq!(
        &reference_cipher().decrypt(&receive(&server)).unwrap()[..2],
        &[0, 5]
    );
}

#[test]
fn continuum_session_requests_map_with_file_index() {
    use aicontinuum::net::session::{Config, Session};
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    server.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    let mut client = Session::connect_continuum(Config {
        server: server.local_addr().unwrap(), name: "fixture".into(), password: "fixture".into(),
        arena: None, map_dir: std::env::temp_dir().join("aic-missing-fixture-map"),
    }, |_| Some([0; 20]), [0; 64]).unwrap();
    let mut buf = [0; 2048];
    let (_, addr) = server.recv_from(&mut buf).unwrap();
    server.send_to(&RESPONSE, addr).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    client.update();
    assert_eq!(receive(&server), ACK);
    let cipher = reference_cipher();
    server.send_to(&cipher.encrypt(&[0, 2]).unwrap(), addr).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    client.update();
    // Drain sync and login before the simulated server accepts the login.
    for _ in 0..2 { receive(&server); }
    let mut response = vec![0, 3, 0, 0, 0, 0, 0x0a, 0];
    server.send_to(&cipher.encrypt(&response).unwrap(), addr).unwrap();
    response = vec![0, 3, 1, 0, 0, 0, 0x29];
    response.extend_from_slice(b"fixture-map.lvl\0");
    response.extend_from_slice(&0x12345678u32.to_le_bytes());
    response.extend_from_slice(&1234u32.to_le_bytes());
    server.send_to(&cipher.encrypt(&response).unwrap(), addr).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    client.update();
    let mut found = false;
    let mut found_arena = false;
    for _ in 0..3 {
        let packet = cipher.decrypt(&receive(&server)).unwrap();
        if packet.starts_with(&[0, 3]) && packet.get(6) == Some(&0x01) {
            let arena = &packet[6..];
            assert_eq!(arena.len(), 27, "Continuum arena entry includes optional graphics byte");
            assert_eq!(arena[26], 0, "optional LVZ downloads are not implemented");
            found_arena = true;
        }
        if packet.starts_with(&[0, 3]) && packet.get(6) == Some(&0x0c) {
            assert_eq!(&packet[6..], &[0x0c, 0, 0]);
            found = true;
        }
    }
    assert!(found, "missing indexed Continuum map request");
    assert!(found_arena, "missing Continuum arena entry");
}

#[test]
fn vie_arena_entry_keeps_the_26_byte_layout() {
    use aicontinuum::net::session::{Config, Session};
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    server.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    let mut client = Session::connect(Config {
        server: server.local_addr().unwrap(), name: "fixture".into(), password: "fixture".into(),
        arena: None, map_dir: std::env::temp_dir().join("aic-vie-arena-fixture"),
    }).unwrap();
    let mut buf = [0; 2048];
    let (_, addr) = server.recv_from(&mut buf).unwrap();
    let mut ack = vec![0, 2];
    ack.extend_from_slice(&buf[2..6]); // echoed VIE key disables encryption
    server.send_to(&ack, addr).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    client.update();
    for _ in 0..2 { receive(&server); } // sync and login
    server.send_to(&[0, 3, 0, 0, 0, 0, 0x0a, 0], addr).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    client.update();
    assert_eq!(&receive(&server)[..2], &[0, 4]);
    let packet = receive(&server);
    assert_eq!(&packet[..2], &[0, 3]);
    assert_eq!(&packet[6..8], &[1, 8]);
    assert_eq!(packet[6..].len(), 26);
    client.disconnect();
}

#[test]
fn questionnaire_requests_are_reported_before_arena_entry() {
    use aicontinuum::net::session::{Config, Phase, Session};

    // Login OK can still require the form. Rejected passwords must retain their
    // own error even if byte 19 is set; short synthetic replies remain supported.
    for (code, flag, expected) in [
        (0x00, Some(1), "questionnaire"),
        (0x0D, Some(2), "questionnaire"),
        (0x06, Some(1), "questionnaire"),
        (0x10, Some(0), "questionnaire"),
        (0x10, None, "questionnaire"),
        (0x02, Some(1), "invalid password"),
        (0x00, Some(0), "arena"),
        (0x00, None, "arena"),
    ] {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        server.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let mut client = Session::connect_continuum(Config {
            server: server.local_addr().unwrap(), name: "fixture".into(), password: "fixture".into(),
            arena: None, map_dir: std::env::temp_dir().join("aic-questionnaire-fixture"),
        }, |_| Some([0; 20]), [0; 64]).unwrap();
        let (_, addr) = server.recv_from(&mut [0; 2048]).unwrap();
        server.send_to(&RESPONSE, addr).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        client.update();
        assert_eq!(receive(&server), ACK);
        let cipher = reference_cipher();
        server.send_to(&cipher.encrypt(&[0, 2]).unwrap(), addr).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        client.update();
        for _ in 0..2 { receive(&server); } // sync, login

        let mut login = vec![0x0a, code];
        if let Some(flag) = flag {
            login.resize(36, 0);
            login[19] = flag;
        }
        // A later JoinGame in the same poll must not overwrite the form error.
        let mut packet = vec![0, 0x0e, (login.len() + 6) as u8, 0, 3, 0, 0, 0, 0];
        packet.extend_from_slice(&login);
        packet.extend_from_slice(&[7, 0, 3, 1, 0, 0, 0, 0x02]);
        server.send_to(&cipher.encrypt(&packet).unwrap(), addr).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        client.update();
        if expected == "arena" {
            assert_eq!(client.phase, Phase::InArena);
        } else {
            assert!(matches!(&client.phase, Phase::Failed(reason) if reason.contains(expected)),
                "code {code}, flag {flag:?}: {:?}", client.phase);
            client.update();
            assert!(matches!(&client.phase, Phase::Failed(reason) if reason.contains(expected)));
        }

        // Decrypt all responses to check the actual wire behavior, not just UI state.
        server.set_nonblocking(true).unwrap();
        let mut saw_arena = false;
        let mut buf = [0; 2048];
        while let Ok(n) = server.recv(&mut buf) {
            let decoded = cipher.decrypt(&buf[..n]).unwrap();
            if decoded.starts_with(&[0, 3]) {
                assert_ne!(decoded.get(6), Some(&0x17), "must not fabricate form answers");
                saw_arena |= decoded.get(6) == Some(&0x01);
            }
        }
        assert_eq!(saw_arena, expected == "arena");
        client.disconnect();
    }
}

#[test]
fn large_encrypted_download_crosses_multiple_ack_windows() {
    let (mut client, server, addr) = pair(|_| Some([0; 20]));
    let cipher = reference_cipher();
    server.send_to(&RESPONSE, addr).unwrap();
    settle(&mut client);
    assert_eq!(receive(&server), ACK);
    server.send_to(&cipher.encrypt(&[0, 2]).unwrap(), addr).unwrap();
    assert!(settle(&mut client).contains(&Event::Connected));
    receive(&server); // initial sync
    let mut whole: Vec<u8> = (0..14_400).map(|i| (i % 251) as u8).collect();
    whole[0] = 0x2a;
    for batch in 0..3u32 {
        // Reorder the first two chunks of each window to exercise reliable buffering.
        let order = [1, 0, 2, 3, 4, 5, 6, 7, 8, 9];
        for offset in order {
            let id = batch * 10 + offset;
            let mut packet = vec![0, 3];
            packet.extend_from_slice(&id.to_le_bytes());
            packet.extend_from_slice(&[0, 0x0a]);
            packet.extend_from_slice(&(whole.len() as u32).to_le_bytes());
            let start = id as usize * 480;
            packet.extend_from_slice(&whole[start..start + 480]);
            server.send_to(&cipher.encrypt(&packet).unwrap(), addr).unwrap();
        }
        let events = settle(&mut client);
        if batch < 2 { assert!(events.is_empty()); }
        else { assert_eq!(events, vec![Event::Packet(whole.clone())]); }
        let ack = cipher.decrypt(&receive(&server)).unwrap();
        assert_eq!(&ack[..2], &[0, 0x0e]);
        let mut ids = Vec::new();
        for entry in ack[2..].chunks_exact(7) {
            assert_eq!(&entry[..3], &[6, 0, 4]);
            ids.push(u32::from_le_bytes(entry[3..7].try_into().unwrap()));
        }
        ids.sort_unstable();
        assert_eq!(ids, (batch * 10..batch * 10 + 10).collect::<Vec<_>>());
        if batch == 0 {
            // Pause after 4,800 bytes until the client answers a padded challenge.
            // This reproduces a server that closes a download when the reply is absent.
            let challenge = [0, 0x0e, 13, 0, 0x12, 123, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7];
            server.send_to(&cipher.encrypt(&challenge).unwrap(), addr).unwrap();
            assert!(settle(&mut client).is_empty());
            let reply = cipher.decrypt(&receive(&server)).unwrap();
            assert_eq!(&reply[..8], &[0, 0x13, 0, 0, 123, 0, 0, 0]);
            assert_eq!(&reply[8..], &[0; 80]);
        }
    }
}

#[test]
fn delayed_download_recovers_lost_chunk_and_deduplicates_retransmission() {
    let (mut client, server, addr) = pair(|_| Some([0; 20]));
    let cipher = reference_cipher();
    server.send_to(&RESPONSE, addr).unwrap();
    settle(&mut client);
    assert_eq!(receive(&server), ACK);
    server.send_to(&cipher.encrypt(&[0, 2]).unwrap(), addr).unwrap();
    assert_eq!(settle(&mut client), vec![Event::Connected]);
    receive(&server);
    let mut whole = vec![0x55; 1440];
    whole[0] = 0x2a;
    let send = |id: u32| {
        let mut packet = vec![0, 3];
        packet.extend_from_slice(&id.to_le_bytes());
        packet.extend_from_slice(&[0, 0x0a]);
        packet.extend_from_slice(&(whole.len() as u32).to_le_bytes());
        packet.extend_from_slice(&whole[id as usize * 480..(id as usize + 1) * 480]);
        server.send_to(&cipher.encrypt(&packet).unwrap(), addr).unwrap();
    };
    // First chunk is lost; later chunks must be acknowledged but not delivered.
    for id in [1, 2, 1] { send(id); }
    assert!(settle(&mut client).is_empty());
    let ack = cipher.decrypt(&receive(&server)).unwrap();
    let ids: Vec<_> = ack[2..].chunks_exact(7)
        .map(|p| u32::from_le_bytes(p[3..7].try_into().unwrap())).collect();
    assert_eq!(ids, vec![1, 2, 1]);
    std::thread::sleep(Duration::from_millis(600));
    assert!(client.poll().is_empty());
    send(0); // server retransmits the missing chunk
    assert_eq!(settle(&mut client), vec![Event::Packet(whole.clone())]);
    assert_eq!(cipher.decrypt(&receive(&server)).unwrap(), vec![0, 4, 0, 0, 0, 0]);
    send(0); // lost acknowledgement causes a duplicate retransmission
    assert!(settle(&mut client).is_empty());
    assert_eq!(cipher.decrypt(&receive(&server)).unwrap(), vec![0, 4, 0, 0, 0, 0]);
}
