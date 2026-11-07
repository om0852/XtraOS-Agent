//! Minimal Win32 security FFI used by the IPC layer: current user SID, a user-only security
//! descriptor for the pipe, reading a pipe's DACL back (tests), and OS randomness.
//! Declared by hand (no extra crate) and checked against Microsoft docs:
//! - ConvertStringSecurityDescriptorToSecurityDescriptorW:
//!   https://learn.microsoft.com/windows/win32/api/sddl/nf-sddl-convertstringsecuritydescriptortosecuritydescriptorw
//! - GetTokenInformation(TokenUser): https://learn.microsoft.com/windows/win32/api/securitybaseapi/nf-securitybaseapi-gettokeninformation
//! - ConvertSidToStringSidW: https://learn.microsoft.com/windows/win32/api/sddl/nf-sddl-convertsidtostringsidw
//! - GetNamedSecurityInfoW: https://learn.microsoft.com/windows/win32/api/aclapi/nf-aclapi-getnamedsecurityinfow
//! - BCryptGenRandom: https://learn.microsoft.com/windows/win32/api/bcrypt/nf-bcrypt-bcryptgenrandom
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::ffi::c_void;
use std::io;
use std::ptr::null_mut;

type BOOL = i32;
type HANDLE = *mut c_void;

const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER_CLASS: u32 = 1; // TOKEN_INFORMATION_CLASS::TokenUser
const SDDL_REVISION_1: u32 = 1;
const SE_FILE_OBJECT: u32 = 1;
const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;

#[repr(C)]
struct SID_AND_ATTRIBUTES {
    Sid: *mut c_void,
    Attributes: u32,
}

#[repr(C)]
pub struct SECURITY_ATTRIBUTES {
    pub nLength: u32,
    pub lpSecurityDescriptor: *mut c_void,
    pub bInheritHandle: BOOL,
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn GetTokenInformation(
        token: HANDLE,
        class: u32,
        info: *mut c_void,
        len: u32,
        ret: *mut u32,
    ) -> BOOL;
    fn ConvertSidToStringSidW(sid: *mut c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl: *const u16,
        revision: u32,
        sd: *mut *mut c_void,
        size: *mut u32,
    ) -> BOOL;
    fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
        sd: *mut c_void,
        revision: u32,
        info: u32,
        out: *mut *mut u16,
        len: *mut u32,
    ) -> BOOL;
    fn GetNamedSecurityInfoW(
        name: *const u16,
        object_type: u32,
        info: u32,
        owner: *mut *mut c_void,
        group: *mut *mut c_void,
        dacl: *mut *mut c_void,
        sacl: *mut *mut c_void,
        sd: *mut *mut c_void,
    ) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> HANDLE;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(mem: *mut c_void) -> *mut c_void;
}

#[link(name = "bcrypt")]
extern "system" {
    fn BCryptGenRandom(alg: *mut c_void, buf: *mut u8, len: u32, flags: u32) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn from_wide(p: *const u16) -> String {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
}

/// String SID of the user running this process, e.g. `S-1-5-21-...-1001`.
pub fn current_user_sid() -> io::Result<String> {
    user_sid_of_process(unsafe { GetCurrentProcess() })
}

/// String SID of the user a process runs as (handle needs PROCESS_QUERY_LIMITED_INFORMATION).
/// The handle is only passed to Win32, which validates it.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn user_sid_of_process(process: *mut c_void) -> io::Result<String> {
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut needed = 0u32;
        GetTokenInformation(token, TOKEN_USER_CLASS, null_mut(), 0, &mut needed);
        // u64 buffer: 8-byte aligned for the SID_AND_ATTRIBUTES cast below (ADR 0041).
        let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
        let ok = GetTokenInformation(
            token,
            TOKEN_USER_CLASS,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        );
        CloseHandle(token);
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let user = &*(buf.as_ptr() as *const SID_AND_ATTRIBUTES);
        let mut s: *mut u16 = null_mut();
        if ConvertSidToStringSidW(user.Sid, &mut s) == 0 {
            return Err(io::Error::last_os_error());
        }
        let out = from_wide(s);
        LocalFree(s.cast());
        Ok(out)
    }
}

/// SDDL for the core pipe: protected DACL, deny NETWORK logons, allow only `sid`.
pub fn pipe_sddl(sid: &str) -> String {
    format!("D:P(D;;GA;;;NU)(A;;GA;;;{sid})")
}

/// Owns a self-relative security descriptor + SECURITY_ATTRIBUTES that point at it.
pub struct OwnedSecurityAttributes {
    sd: *mut c_void,
    pub attrs: SECURITY_ATTRIBUTES,
}

// The descriptor is immutable after creation and only read by CreateNamedPipeW.
unsafe impl Send for OwnedSecurityAttributes {}
unsafe impl Sync for OwnedSecurityAttributes {}

impl OwnedSecurityAttributes {
    pub fn from_sddl(sddl: &str) -> io::Result<Self> {
        let w = wide(sddl);
        let mut sd: *mut c_void = null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                w.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            sd,
            attrs: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd,
                bInheritHandle: 0,
            },
        })
    }

    pub fn as_mut_ptr(&mut self) -> *mut c_void {
        (&mut self.attrs as *mut SECURITY_ATTRIBUTES).cast()
    }
}

impl Drop for OwnedSecurityAttributes {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.sd);
        }
    }
}

/// Reads the DACL of a named object (e.g. `\\.\pipe\name`) back as SDDL.
pub fn dacl_sddl_of(path: &str) -> io::Result<String> {
    let w = wide(path);
    unsafe {
        let mut sd: *mut c_void = null_mut();
        let rc = GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut sd,
        );
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc as i32));
        }
        let mut s: *mut u16 = null_mut();
        let ok = ConvertSecurityDescriptorToStringSecurityDescriptorW(
            sd,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut s,
            null_mut(),
        );
        LocalFree(sd);
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let out = from_wide(s);
        LocalFree(s.cast());
        Ok(out)
    }
}

/// Cryptographically secure random bytes from the OS.
pub fn random_bytes(n: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    let status = unsafe {
        BCryptGenRandom(
            null_mut(),
            buf.as_mut_ptr(),
            n as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        return Err(io::Error::other(format!(
            "BCryptGenRandom failed: 0x{status:08x}"
        )));
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sid_looks_like_a_user_sid() {
        let sid = current_user_sid().unwrap();
        assert!(sid.starts_with("S-1-5-"), "{sid}");
    }

    #[test]
    fn sddl_round_trips_through_the_os() {
        let sid = current_user_sid().unwrap();
        let sa = OwnedSecurityAttributes::from_sddl(&pipe_sddl(&sid)).unwrap();
        assert!(!sa.attrs.lpSecurityDescriptor.is_null());
    }

    #[test]
    fn random_bytes_are_random() {
        let a = random_bytes(32).unwrap();
        let b = random_bytes(32).unwrap();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
