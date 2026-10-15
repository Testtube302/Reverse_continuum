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

    for early in [&[0x03, 7][..], &[0, 2][..], &[0, 3, 0, 0, 0, 0, 9][..]] {
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
fn rejects_invalid_encrypted_acknowledgement() {
    let (mut client, server, addr) = pair(|_| Some([0; 20]));
    server.send_to(&RESPONSE, addr).unwrap();
    assert!(settle(&mut client).is_empty());
    assert_eq!(receive(&server), ACK);
    server
        .send_to(
            &reference_cipher().encrypt(&[0, 2, 1, 0, 0, 0, 0]).unwrap(),
            addr,
        )
        .unwrap();
    assert_eq!(
        settle(&mut client),
        vec![Event::Disconnected(
            "server sent an invalid Continuum encryption acknowledgement".into()
        )]
    );
    no_packet(&server);
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
    assert_eq!(&response[..6], &[0, 0x13, 123, 0, 0, 0]);
    assert_eq!(&response[6..], &[1, 0xef, 0xcd, 0xab].repeat(20));
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
    assert_eq!(&login[101..], &[0x5a; 64]);
    session.disconnect();
}
