//! Headless controlled-server login check using a local expansion helper.
use aicontinuum::net::continuum_expander::CommandExpander;
use aicontinuum::net::session::{Config, Phase, Session};
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(3..=4).contains(&args.len()) {
        return Err(
            "usage: continuum_login <server-ip:port> <expander-executable> [hold-seconds]".into(),
        );
    }
    let mut expander = CommandExpander::new(args[2].clone().into());
    let config = Config {
        server: args[1].parse()?,
        name: "AIC-ContTest".into(),
        password: "local-test".into(),
        arena: None,
        map_dir: std::env::temp_dir().join("aicontinuum-continuum-test-maps"),
    };
    let identity = aicontinuum::net::native_identity::prepare(|nonce| expander.expand_identity(nonce))?;
    let mut session = Session::connect_continuum_identified(
        config,
        move |seed| match expander.expand(seed) {
            Ok(table) => Some(table),
            Err(error) => {
                eprintln!("Expansion failed: {error}");
                None
            }
        },
        identity,
    )?;
    let hold: u64 = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(0);
    let mut ready_at = None;
    let deadline = Instant::now() + Duration::from_secs(20 + hold);
    loop {
        session.update();
        if session.ready() {
            if ready_at.is_none() {
                println!(
                    "Continuum encrypted login and arena entry verified; map and settings received."
                );
                ready_at = Some(Instant::now());
            }
            if ready_at.unwrap().elapsed() >= Duration::from_secs(hold) {
                println!(
                    "Session held for {hold}s; {} security replies sent.",
                    session.security_replies
                );
                session.disconnect();
                return Ok(());
            }
            session.send_position(None, Default::default(), false, false);
        }
        if let Phase::Failed(reason) = &session.phase {
            return Err(format!("{reason}; log: {:?}", session.log).into());
        }
        if Instant::now() > deadline {
            session.disconnect();
            return Err(format!("login timed out; log: {:?}", session.log).into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
