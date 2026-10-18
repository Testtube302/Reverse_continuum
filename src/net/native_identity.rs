//! Native platform information for Continuum login.
//!
//! Platform-native mapping; unavailable legacy fields remain zero. This is not
//! a reproduction of all original Windows metadata. Server acceptance still
//! requires native runtime testing. Identifiers must never be logged.
use super::continuum_identity;
use super::continuum_identity::LoginIdentity;
use std::io;

/// Collect current OS information and encode it with a fresh per-session nonce.
/// No account name, captured identity or user-selected identifier is accepted.
pub fn prepare(
    expand_identity: impl FnOnce(u32) -> io::Result<[u32; 20]>,
) -> io::Result<LoginIdentity> {
    let (machine_id, timezone_bias, fields) = platform::collect()?;
    let nonce = random_nonce()?;
    let key = expand_identity(nonce)?;
    Ok(LoginIdentity {
        machine_id,
        timezone_bias,
        billing: continuum_identity::encode(nonce, &fields, key),
    })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn random_nonce() -> io::Result<u32> {
    use std::io::Read;
    let mut bytes = [0; 4];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

#[cfg(target_os = "windows")]
fn random_nonce() -> io::Result<u32> { platform::random_nonce() }

#[cfg(target_os = "windows")]
#[path = "native_identity_windows.rs"]
mod platform;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod platform {
    use crate::lvl::crc32;
    use std::ffi::CStr;
    use std::io;

    pub(super) fn collect() -> io::Result<(u32, i16, [u32; 14])> {
        let user = username()?;
        let host = hostname()?;
        #[cfg(target_os = "macos")]
        let machine = volume_machine_id(&volume_uuid()?);
        #[cfg(target_os = "linux")]
        let machine = linux_machine_id()?;
        let mut fields = [0; 14];
        // Original 0x4401e0 hashes these strings without their NUL terminators
        // and leaves the running CRC uncomplemented.
        fields[11] = !crc32(&user);
        fields[12] = !crc32(&host);
        Ok((machine, timezone_bias()?, fields))
    }

    fn username() -> io::Result<Vec<u8>> {
        let mut size = 1024;
        loop {
            let mut storage = vec![0u8; size];
            // SAFETY: libc initializes pwd and result, and any returned strings
            // refer to storage, which remains alive until copied below.
            let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
            let mut result = std::ptr::null_mut();
            let code = unsafe {
                libc::getpwuid_r(
                    libc::geteuid(),
                    &mut pwd,
                    storage.as_mut_ptr().cast(),
                    storage.len(),
                    &mut result,
                )
            };
            if code == libc::ERANGE && size < 65536 {
                size *= 2;
                continue;
            }
            if code != 0 {
                return Err(io::Error::from_raw_os_error(code));
            }
            if result.is_null() || pwd.pw_name.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "local user name is unavailable",
                ));
            }
            // SAFETY: successful getpwuid_r returns a terminated name in storage.
            let value = unsafe { CStr::from_ptr(pwd.pw_name) }.to_bytes().to_vec();
            if value.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "local user name is empty",
                ));
            }
            return Ok(value);
        }
    }

    fn hostname() -> io::Result<Vec<u8>> {
        let mut bytes = [0u8; 256];
        // SAFETY: bytes is writable for the supplied capacity.
        if unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let end = bytes
            .iter()
            .position(|&b| b == 0)
            .filter(|&end| end > 0)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "local host name is empty or truncated",
                )
            })?;
        Ok(bytes[..end].to_vec())
    }

    #[cfg(target_os = "macos")]
    fn volume_uuid() -> io::Result<[u8; 16]> {
        // The Data volume survives sealed-system snapshots and OS updates.
        // Older macOS installations have a single root volume instead.
        let path = if std::path::Path::new("/System/Volumes/Data").is_dir() {
            c"/System/Volumes/Data"
        } else {
            c"/"
        };
        let mut attrs = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: 0,
            volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_UUID,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        };
        // Four-byte length followed by a 16-byte UUID, with four-byte alignment.
        let mut words = [0u32; 5];
        // SAFETY: attrs describes exactly the fields fitting in words; the path
        // is NUL-terminated. No pointers or variable-length attributes are used.
        if unsafe {
            libc::getattrlist(
                path.as_ptr(),
                (&mut attrs as *mut libc::attrlist).cast(),
                words.as_mut_ptr().cast(),
                20,
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if words[0] != 20 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected volume identity length",
            ));
        }
        let mut uuid = [0u8; 16];
        for (chunk, word) in uuid.chunks_exact_mut(4).zip(&words[1..]) {
            chunk.copy_from_slice(&word.to_ne_bytes());
        }
        if uuid == [0; 16] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "volume UUID is unavailable",
            ));
        }
        Ok(uuid)
    }

    #[cfg(target_os = "macos")]
    fn volume_machine_id(uuid: &[u8; 16]) -> u32 {
        // Native mapping of a real volume identity into the legacy 32-bit field.
        // Match the original's signed-absolute-value convention and zero fallback.
        (crc32(uuid) as i32).unsigned_abs().max(1)
    }

    #[cfg(target_os = "linux")]
    fn linux_machine_id() -> io::Result<u32> {
        let id = std::fs::read_to_string("/etc/machine-id")
            .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))?;
        let id = id.trim();
        if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) || id.bytes().all(|b| b == b'0') {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "local machine identity is unavailable"));
        }
        // Native mapping into the protocol's 32-bit field; never transmit the raw ID.
        Ok((crc32(format!("AIContinuum:{}", id.to_ascii_lowercase()).as_bytes()) as i32).unsigned_abs().max(1))
    }

    fn timezone_bias() -> io::Result<i16> {
        // SAFETY: time accepts a null output pointer; localtime_r initializes tm.
        let now = unsafe { libc::time(std::ptr::null_mut()) };
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
            return Err(io::Error::other("local timezone is unavailable"));
        }
        i16::try_from(-tm.tm_gmtoff / 60).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "timezone bias is out of range")
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn native_information_is_stable_and_unsupported_fields_stay_absent() {
            let (machine, bias, fields) = collect().unwrap();
            let second = collect().unwrap();
            // Boolean assertions avoid printing actual identification values.
            assert!(machine == second.0 && fields == second.2);
            assert!(machine != 0 && (-1440..=1440).contains(&bias));
            assert!(fields[..11].iter().all(|&word| word == 0) && fields[13] == 0);
            assert!(fields[11] == !crc32(&username().unwrap()));
            assert!(fields[12] == !crc32(&hostname().unwrap()));
        }
    }
}
