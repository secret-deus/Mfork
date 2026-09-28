//! Fixed known-folder installation with a protected, narrowly allowlisted DACL.
//! Never repairs/adopts a pre-created user-owned directory. No shell ACL commands.
#![allow(dead_code)]
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath};

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}
fn text_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn last_error() -> String {
    std::io::Error::last_os_error().to_string()
}
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}

pub fn install_root() -> Result<PathBuf, String> {
    let mut p = null_mut();
    let hr = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, 0, null_mut(), &mut p) };
    if hr < 0 {
        return Err(format!("ProgramData known-folder lookup failed: {hr:#x}"));
    }
    if p.is_null() {
        return Err("ProgramData lookup returned no path".into());
    }
    let mut len = 0;
    unsafe {
        while *p.add(len) != 0 {
            len += 1;
        }
    }
    let value = String::from_utf16(unsafe { std::slice::from_raw_parts(p, len) });
    unsafe {
        CoTaskMemFree(p.cast());
    }
    let base = PathBuf::from(value.map_err(|e| e.to_string())?);
    if !base.is_absolute() {
        return Err("ProgramData must be absolute".into());
    }
    // Refuse redirected roots instead of letting an elevated installer traverse them.
    for path in base.ancestors() {
        reject_reparse(path)?;
    }
    Ok(base.join("MforkService"))
}

pub fn reject_reparse(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() || m.file_attributes() & 0x400 != 0 => {
            Err(format!("refusing reparse point: {}", path.display()))
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("inspect {}: {e}", path.display())),
    }
}
fn sid_text(sid: PSID) -> Result<String, String> {
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return Err("invalid SID".into());
    }
    let mut p = null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut p) } == 0 {
        return Err(last_error());
    }
    let _free = Local(p.cast());
    let mut len = 0;
    unsafe {
        while *p.add(len) != 0 {
            len += 1;
        }
    }
    String::from_utf16(unsafe { std::slice::from_raw_parts(p, len) }).map_err(|e| e.to_string())
}
fn user_sid_string(s: &str) -> bool {
    (s.starts_with("S-1-5-21-") || s.starts_with("S-1-12-1-"))
        && s.len() <= 184
        && s[2..].bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
fn validate_user_sid(s: &str) -> Result<(), String> {
    if !user_sid_string(s) {
        return Err("expected installing user's SID, not a broad group or SDDL expression".into());
    }
    let mut sid = null_mut();
    if unsafe { ConvertStringSidToSidW(text_wide(s).as_ptr(), &mut sid) } == 0 {
        return Err(last_error());
    }
    let _free = Local(sid);
    if sid_text(sid)? != s {
        return Err("noncanonical SID".into());
    }
    Ok(())
}
pub fn current_user_sid() -> Result<String, String> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error());
    }
    let result = (|| {
        let mut size = 0;
        unsafe {
            GetTokenInformation(token, TokenUser, null_mut(), 0, &mut size);
        }
        if size == 0 || size > 65536 {
            return Err("invalid TokenUser size".into());
        }
        let mut aligned = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                aligned.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        } == 0
        {
            return Err(last_error());
        }
        let info = unsafe { &*aligned.as_ptr().cast::<TOKEN_USER>() };
        let sid = sid_text(info.User.Sid)?;
        validate_user_sid(&sid)?;
        Ok(sid)
    })();
    unsafe {
        CloseHandle(token);
    }
    result
}
fn descriptor(owner: &str, directory: bool, execute: bool) -> Result<Local, String> {
    validate_user_sid(owner)?;
    let flags = if directory { "OICI" } else { "" };
    let read = if execute { "GRGX" } else { "GR" };
    let sddl =
        format!("O:BAG:BAD:P(A;{flags};FA;;;SY)(A;{flags};FA;;;BA)(A;{flags};{read};;;{owner})");
    let mut sd = null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text_wide(&sddl).as_ptr(),
            1,
            &mut sd,
            null_mut(),
        )
    } == 0
    {
        return Err(last_error());
    }
    Ok(Local(sd))
}
// Generic rights may be mapped by the filesystem. Accept either representation.
const READ_EXECUTE: u32 = 0xA0000000 | 0x001200A9;
fn read_only_mask(mask: u32) -> bool {
    mask != 0 && mask & !READ_EXECUTE == 0
}
fn full_mask(mask: u32) -> bool {
    mask == 0x10000000 || mask == 0x001F01FF
}

/// Return the single authorized SID. Reject NULL/unprotected DACLs, inherited or
/// non-allow ACEs, broad reader groups, unknown writable entries and missing admins.
fn inspect_acl(path: &Path, directory: bool) -> Result<String, String> {
    reject_reparse(path)?;
    let mut sd = null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide(path.as_os_str()).as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut sd,
        )
    };
    if status != 0 {
        return Err(format!(
            "read ACL {}: {}",
            path.display(),
            std::io::Error::from_raw_os_error(status as i32)
        ));
    }
    let _free = Local(sd);
    inspect_descriptor(sd, directory)
}
fn inspect_descriptor(sd: PSECURITY_DESCRIPTOR, directory: bool) -> Result<String, String> {
    let mut owner = null_mut();
    let mut acl = null_mut();
    let mut present = 0;
    let mut defaulted = 0;
    if unsafe { GetSecurityDescriptorOwner(sd, &mut owner, &mut defaulted) } == 0
        || unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut acl, &mut defaulted) } == 0
        || present == 0
    {
        return Err(last_error());
    }
    if !["S-1-5-18", "S-1-5-32-544"].contains(&sid_text(owner)?.as_str()) {
        return Err("service path is not SYSTEM/Administrators owned; refusing adoption".into());
    }
    let mut control = 0;
    let mut revision = 0;
    if unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) } == 0
        || control & SE_DACL_PROTECTED == 0
        || acl.is_null()
    {
        return Err("service path must have a protected non-null DACL".into());
    }
    if unsafe { (*acl).AceCount } != 3 {
        return Err("unexpected service ACL entries; refusing adoption".into());
    }
    let mut system = false;
    let mut admin = false;
    let mut reader = None;
    for i in 0..3 {
        let mut raw = null_mut();
        if unsafe { GetAce(acl, i, &mut raw) } == 0 || raw.is_null() {
            return Err(last_error());
        }
        let header = unsafe { &*raw.cast::<ACE_HEADER>() };
        let flags = if directory { 3 } else { 0 }; // OBJECT_INHERIT | CONTAINER_INHERIT
        if header.AceType != 0
            || header.AceFlags != flags
            || (header.AceSize as usize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
        {
            return Err("unexpected service ACE type/flags".into());
        }
        let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = (&ace.SidStart as *const u32).cast_mut().cast();
        if unsafe { IsValidSid(sid) } == 0
            || unsafe { GetLengthSid(sid) } as usize > header.AceSize as usize - 8
        {
            return Err("invalid service ACE SID".into());
        }
        let text = sid_text(sid)?;
        match text.as_str() {
            "S-1-5-18" if !system && full_mask(ace.Mask) => system = true,
            "S-1-5-32-544" if !admin && full_mask(ace.Mask) => admin = true,
            _ if reader.is_none() && user_sid_string(&text) && read_only_mask(ace.Mask) => {
                reader = Some(text)
            }
            _ => return Err("service ACL grants unexpected access".into()),
        }
    }
    if !system || !admin {
        return Err("service ACL missing SYSTEM/Administrators access".into());
    }
    reader.ok_or_else(|| "service ACL missing authorized reader".into())
}
pub fn validate_root(root: &Path) -> Result<(), String> {
    if root != install_root()? || !root.is_dir() {
        return Err("invalid protected service root".into());
    }
    inspect_acl(root, true).map(|_| ())
}
pub fn prepare_root(owner_sid: &str) -> Result<PathBuf, String> {
    let root = install_root()?;
    let sd = descriptor(owner_sid, true, true)?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    };
    let created = unsafe { CreateDirectoryW(wide(root.as_os_str()).as_ptr(), &sa) };
    if created == 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
            return Err(e.to_string());
        }
    }
    validate_root(&root)?;
    if inspect_acl(&root, true)? != owner_sid {
        return Err("service belongs to another user; administrator migration required".into());
    }
    Ok(root)
}
pub fn prepare_subdir(root: &Path, name: &str) -> Result<PathBuf, String> {
    if !["core", "data"].contains(&name) {
        return Err("unsupported service subdirectory".into());
    }
    validate_root(root)?;
    let owner = inspect_acl(root, true)?;
    let sd = descriptor(&owner, true, true)?;
    let path = root.join(name);
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    };
    if unsafe { CreateDirectoryW(wide(path.as_os_str()).as_ptr(), &sa) } == 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
            return Err(e.to_string());
        }
    }
    if !path.is_dir() || inspect_acl(&path, true)? != owner {
        return Err("unsafe service subdirectory".into());
    }
    Ok(path)
}
fn restrict(path: &Path, owner_sid: &str, execute: bool) -> Result<(), String> {
    reject_reparse(path)?;
    let sd = descriptor(owner_sid, false, execute)?;
    let mut owner = null_mut();
    let mut defaulted = 0;
    let mut present = 0;
    let mut acl = null_mut();
    if unsafe { GetSecurityDescriptorOwner(sd.0, &mut owner, &mut defaulted) } == 0
        || unsafe { GetSecurityDescriptorDacl(sd.0, &mut present, &mut acl, &mut defaulted) } == 0
        || present == 0
    {
        return Err(last_error());
    }
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide(path.as_os_str()).as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            owner,
            null_mut(),
            acl,
            null_mut(),
        )
    };
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(status as i32).to_string());
    }
    if inspect_acl(path, false)? != owner_sid {
        return Err("service file ACL verification failed".into());
    }
    Ok(())
}
pub fn restrict_file(path: &Path, owner_sid: &str) -> Result<(), String> {
    restrict(path, owner_sid, false)
}
pub fn restrict_image(path: &Path, owner_sid: &str) -> Result<(), String> {
    restrict(path, owner_sid, true)
}
pub fn validate_token(root: &Path) -> Result<(), String> {
    if inspect_acl(&root.join("service.token"), false)? != inspect_acl(root, true)? {
        return Err("token ACL does not match authorized user".into());
    }
    Ok(())
}
pub fn validate_image(root: &Path, path: &Path) -> Result<(), String> {
    if inspect_acl(path, false)? != inspect_acl(root, true)? {
        return Err("image ACL does not match protected installation".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn broad_or_injected_sid_is_rejected() {
        for s in [
            "",
            "S-1-1-0",
            "S-1-5-32-545",
            "S-1-5-18",
            "S-1-5-21-1)(A;;FA;;;WD)",
        ] {
            assert!(!user_sid_string(s));
        }
        assert!(validate_user_sid("S-1-5-21-1-2-3-1001").is_ok());
    }
    #[test]
    fn reader_never_has_write_delete_or_acl_control() {
        assert!(read_only_mask(0x001200A9));
        assert!(read_only_mask(0x80000000));
        for mask in [
            0, 0x10000000, 0x40000000, 0x00010000, 0x00040000, 0x00080000, 2, 4, 0x40, 0x100,
        ] {
            assert!(!read_only_mask(mask));
        }
    }
    #[test]
    fn generated_descriptors_round_trip_without_filesystem_or_elevation() {
        let sid = "S-1-5-21-1-2-3-1001";
        for (directory, execute) in [(true, true), (false, false), (false, true)] {
            let sd = descriptor(sid, directory, execute).unwrap();
            assert_eq!(inspect_descriptor(sd.0, directory).unwrap(), sid);
        }
    }
    #[test]
    fn unprotected_or_world_readable_descriptors_fail_closed() {
        for s in [
            "O:BAG:BAD:(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;S-1-5-21-1-2-3-1001)",
            "O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GRGX;;;WD)",
            "O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;GW;;;S-1-5-21-1-2-3-1001)",
        ] {
            let mut sd = null_mut();
            assert_ne!(
                unsafe {
                    ConvertStringSecurityDescriptorToSecurityDescriptorW(
                        text_wide(s).as_ptr(),
                        1,
                        &mut sd,
                        null_mut(),
                    )
                },
                0
            );
            let sd = Local(sd);
            assert!(inspect_descriptor(sd.0, true).is_err());
        }
    }
}

/// Same-volume atomic replacement. Caller creates the source with its final ACL first.
pub fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    reject_reparse(source)?;
    reject_reparse(target)?;
    if unsafe {
        MoveFileExW(
            wide(source.as_os_str()).as_ptr(),
            wide(target.as_os_str()).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(last_error());
    }
    Ok(())
}

#[cfg(test)]
mod file_tests {
    use super::*;
    #[test]
    fn atomic_replacement_preserves_target_on_failure() {
        let base = std::env::var_os("PI_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!(
            "mfork-replace-test-{}",
            super::super::protocol::random_hex().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source");
        let target = root.join("target");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        replace_file(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!source.exists());
        std::fs::write(&source, b"retry").unwrap();
        let directory = root.join("directory");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("sentinel"), b"keep").unwrap();
        assert!(replace_file(&source, &directory).is_err());
        assert_eq!(std::fs::read(directory.join("sentinel")).unwrap(), b"keep");
        std::fs::remove_dir_all(root).unwrap();
    }
}
