//! Observe Continuum negotiation without supplying credentials.
//! Usage: cargo run --example continuum_probe -- 127.0.0.1:5000 [server-scrty1]
//! Exit 2 means a Continuum challenge was received but key expansion is unavailable.
use aicontinuum::net::continuum_cache::ExpansionCache;
use aicontinuum::net::transport::{Event, Transport};
use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let Some(server) = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<SocketAddr>().ok())
    else {
        eprintln!("Usage: continuum_probe <server-ip:port> [server-scrty1]");
        return ExitCode::FAILURE;
    };
    let cache = match std::env::args().nth(2) {
        Some(path) => match std::fs::read(path).and_then(|b| ExpansionCache::parse(&b)) {
            Ok(cache) => Some(cache),
            Err(error) => {
                eprintln!("Could not read expansion cache: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    let challenged = Arc::new(AtomicBool::new(false));
    let observed = challenged.clone();
    let mut transport = match Transport::connect_continuum(server, move |key| {
        observed.store(true, Ordering::Relaxed);
        eprintln!("Received a Continuum encryption challenge.");
        cache.as_ref().and_then(|cache| cache.lookup(key))
    }) {
        Ok(transport) => transport,
        Err(error) => {
            eprintln!("Could not open transport: {error}");
            return ExitCode::FAILURE;
        }
    };
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        for event in transport.poll() {
            if matches!(event, Event::Connected) {
                eprintln!("Continuum encrypted acknowledgement verified. No login attempted.");
                transport.disconnect();
                return ExitCode::SUCCESS;
            }
            if let Event::Disconnected(reason) = event {
                eprintln!("{reason}");
                return if challenged.load(Ordering::Relaxed) {
                    ExitCode::from(2)
                } else {
                    ExitCode::FAILURE
                };
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    transport.disconnect();
    eprintln!("No usable Continuum encryption challenge received before the deadline.");
    ExitCode::FAILURE
}
