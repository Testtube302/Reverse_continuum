//! Outgoing chat checks against a synthetic loopback peer; no zone is needed.
use aicontinuum::lvl::Map;
use aicontinuum::net::session::{Config, Phase, Session};
use aicontinuum::settings::ArenaSettings;
use std::net::UdpSocket;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn outgoing_chat_displays_the_wire_text_with_our_player_id() {
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    let mut session = Session::connect(Config {
        server: peer.local_addr().unwrap(), name: "ChatFixture".into(), password: "test".into(),
        arena: None, map_dir: std::env::temp_dir(),
    }).unwrap();
    let mut buf = [0; 2048];
    let (_, addr) = peer.recv_from(&mut buf).unwrap();
    session.send_public_chat("Too early");
    assert!(session.chat.is_empty(), "no phantom message before arena entry");
    let mut ack = vec![0, 2];
    ack.extend_from_slice(&buf[2..6]); // Echoed VIE key selects unencrypted traffic.
    peer.send_to(&ack, addr).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    session.update();
    for _ in 0..2 { peer.recv(&mut buf).unwrap(); } // Sync and login.
    session.phase = Phase::InArena;
    session.joined = true;
    session.pid = Some(42);
    session.map = Some(Arc::new(Map::empty()));
    session.settings = Some(Arc::new(ArenaSettings::default()));

    session.send_public_chat("");
    assert!(session.chat.is_empty());
    for (input, expected) in [
        ("Hello arena".to_owned(), "Hello arena".to_owned()),
        ("x".repeat(260), "x".repeat(240)),
        ("Visible\0hidden".to_owned(), "Visible".to_owned()),
    ] {
        let before = session.chat.len();
        session.send_public_chat(&input);
        let n = peer.recv(&mut buf).unwrap();
        assert_eq!(&buf[..2], &[0, 3]); // Reliable envelope.
        assert_eq!(&buf[6..11], &[0x06, 2, 0, 0, 0]); // Public chat, no target.
        assert_eq!(&buf[11..n - 1], &input.as_bytes()[..input.len().min(240)]);
        assert_eq!(buf[n - 1], 0);
        assert_eq!(session.chat.len(), before + 1);
        let shown = session.chat.last().unwrap();
        assert_eq!((shown.kind, shown.pid), (2, 42));
        assert_eq!(shown.text, expected);
    }
    session.disconnect();
}
