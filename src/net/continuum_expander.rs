//! Optional local expansion helper. No shell or network service is used.
//! Protocol: decimal key2 argument; exactly 80 binary bytes on stdout.
//! Identity expansion adds `--identity` and requires a supporting helper.
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct CommandExpander {
    program: PathBuf,
}

impl CommandExpander {
    pub fn new(program: PathBuf) -> Self {
        Self { program }
    }

    pub fn expand(&mut self, seed: u32) -> io::Result<[u32; 20]> {
        self.run(seed, false)
    }

    /// Requires a helper supporting the explicit `--identity` mode. Never fall
    /// back to the session expansion: its extra stage produces a different key.
    pub fn expand_identity(&mut self, nonce: u32) -> io::Result<[u32; 20]> {
        self.run(nonce, true)
    }

    fn run(&self, seed: u32, identity: bool) -> io::Result<[u32; 20]> {
        let mut command = Command::new(&self.program);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW; keep stdout pipe for the table.
        }
        // Windows bundles a private CPython interpreter instead of a frozen
        // PyInstaller host (Unicorn #2281). No PATH/system Python is consulted.
        if self.program.file_name().is_some_and(|name| name == "python.exe") {
            command.arg("-I").arg("-B").arg(self.program.with_file_name("continuum_reference_expander.py"));
        }
        command.arg(seed.to_string());
        if identity {
            command.arg("--identity");
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Drain stderr concurrently: a Python loader traceback can exceed a pipe buffer.
        let mut stderr = child.stderr.take().expect("piped stderr");
        let errors = std::thread::spawn(move || {
            let mut kept = Vec::new();
            let mut buffer = [0; 1024];
            while let Ok(count) = stderr.read(&mut buffer) {
                if count == 0 { break; }
                let keep = count.min(8192usize.saturating_sub(kept.len()));
                kept.extend_from_slice(&buffer[..keep]);
            }
            String::from_utf8_lossy(&kept).trim().to_string()
        });
        // Identity runs before network login; allow cold interpreter/security scanning.
        // In-session challenges retain the short response deadline.
        let deadline = Instant::now() + Duration::from_secs(if identity { 15 } else { 5 });
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                outcome => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = errors.join();
                    return Err(outcome.err().unwrap_or_else(|| {
                        io::Error::new(io::ErrorKind::TimedOut, "key expansion helper timed out")
                    }));
                }
            }
        };
        let detail = errors.join().unwrap_or_default();
        if !status.success() {
            return Err(io::Error::other(format!("key expansion helper failed ({status}): {detail}")));
        }
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .take(81)
            .read_to_end(&mut bytes)?;
        decode_table(&bytes)
    }
}

fn decode_table(bytes: &[u8]) -> io::Result<[u32; 20]> {
    if bytes.len() != 80 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "expansion helper must return exactly 80 bytes",
        ));
    }
    Ok(std::array::from_fn(|i| {
        u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn preserves_helper_failure_reason_without_pipe_deadlock() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("aic-expander-error-{}.sh", std::process::id()));
        std::fs::write(&path, "#!/bin/sh\necho 'missing runtime fixture' >&2\ni=0; while [ $i -lt 2000 ]; do echo diagnostic >&2; i=$((i+1)); done\nexit 7\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let error = CommandExpander::new(path.clone()).expand(0).unwrap_err().to_string();
        std::fs::remove_file(path).unwrap();
        assert!(error.contains("missing runtime fixture"));
        assert!(error.len() < 8400);
    }

    #[test]
    fn requires_exact_binary_table() {
        for size in [0, 79, 81, 160] {
            assert!(decode_table(&vec![0; size]).is_err());
        }
        let bytes: Vec<_> = (0..20u32).flat_map(u32::to_le_bytes).collect();
        assert_eq!(
            decode_table(&bytes).unwrap(),
            std::array::from_fn(|i| i as u32)
        );
    }
}
