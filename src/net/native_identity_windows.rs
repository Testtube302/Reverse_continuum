//! Windows APIs documented by Microsoft Learn; see docs/PLATFORMS.md.
use crate::lvl::crc32;
use std::io;
use std::ptr::{null, null_mut};

#[repr(C)]
struct TimeZoneInformation {
    bias: i32,
    standard_name: [u16; 32],
    standard_date: [u16; 8],
    standard_bias: i32,
    daylight_name: [u16; 32],
    daylight_date: [u16; 8],
    daylight_bias: i32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetComputerNameA(buffer: *mut u8, size: *mut u32) -> i32;
    fn GetWindowsDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn GetVolumePathNameW(file: *const u16, volume: *mut u16, size: u32) -> i32;
    fn GetVolumeInformationW(root: *const u16, name: *mut u16, size: u32, serial: *mut u32,
        max_component: *mut u32, flags: *mut u32, filesystem: *mut u16, fs_size: u32) -> i32;
    fn GetTimeZoneInformation(zone: *mut TimeZoneInformation) -> u32;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn GetUserNameA(buffer: *mut u8, size: *mut u32) -> i32;
}
#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(algorithm: *const std::ffi::c_void, buffer: *mut u8, size: u32, flags: u32) -> i32;
}

fn name(api: unsafe extern "system" fn(*mut u8, *mut u32) -> i32) -> io::Result<Vec<u8>> {
    let mut bytes = [0u8; 1024];
    let mut len = bytes.len() as u32;
    // SAFETY: both APIs receive the actual writable buffer capacity and length pointer.
    if unsafe { api(bytes.as_mut_ptr(), &mut len) } == 0 { return Err(io::Error::last_os_error()); }
    let end = bytes.iter().position(|&b| b == 0).filter(|&n| n > 0)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "local name is empty or truncated"))?;
    Ok(bytes[..end].to_vec())
}

pub(super) fn collect() -> io::Result<(u32, i16, [u32; 14])> {
    let user = name(GetUserNameA)?;
    let host = name(GetComputerNameA)?;
    let mut directory = [0u16; 32768];
    let mut root = [0u16; 32768];
    let mut serial = 0;
    // SAFETY: each output is initialized storage of the specified capacity.
    let len = unsafe { GetWindowsDirectoryW(directory.as_mut_ptr(), directory.len() as u32) };
    if len == 0 { return Err(io::Error::last_os_error()); }
    if len as usize >= directory.len() { return Err(io::Error::other("Windows directory path is too long")); }
    if unsafe { GetVolumePathNameW(directory.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: root is terminated by GetVolumePathNameW; only serial is requested.
    if unsafe { GetVolumeInformationW(root.as_ptr(), null_mut(), 0, &mut serial, null_mut(), null_mut(), null_mut(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: this C-layout structure consists entirely of integers.
    let mut zone: TimeZoneInformation = unsafe { std::mem::zeroed() };
    let state = unsafe { GetTimeZoneInformation(&mut zone) };
    let adjustment = match state {
        0 => 0,
        1 => zone.standard_bias,
        2 => zone.daylight_bias,
        _ => return Err(io::Error::last_os_error()),
    };
    let bias = i16::try_from(zone.bias + adjustment)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "timezone bias is out of range"))?;
    let mut fields = [0; 14];
    fields[11] = !crc32(&user);
    fields[12] = !crc32(&host);
    Ok(((serial as i32).unsigned_abs().max(1), bias, fields))
}

pub(super) fn random_nonce() -> io::Result<u32> {
    let mut bytes = [0; 4];
    // SAFETY: null provider plus flag 2 requests the system RNG; buffer has four bytes.
    let status = unsafe { BCryptGenRandom(null(), bytes.as_mut_ptr(), bytes.len() as u32, 2) };
    if status < 0 { return Err(io::Error::other("Windows system random generator failed")); }
    Ok(u32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_identity_is_stable_without_logging_identifiers() {
        assert_eq!(std::mem::size_of::<TimeZoneInformation>(), 172);
        let first = collect().unwrap();
        let second = collect().unwrap();
        assert!(first == second);
        assert!(first.0 != 0 && (-1440..=1440).contains(&first.1));
        assert!(first.2[..11].iter().all(|&n| n == 0) && first.2[13] == 0);
        random_nonce().unwrap();
    }
}
