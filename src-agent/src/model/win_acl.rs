//! Windows file ACL restriction — restricts files to owner + SYSTEM + Administrators.
//! Equivalent intent to Unix `chmod 0600`.

use std::path::Path;

/// Apply a restrictive DACL to `path`, granting access only to the current user (OW),
/// SYSTEM (SY), and Administrators (BA). The `D:` prefix in SDDL means
/// DACL is present; `P` means protected (no inheritance).
///
/// Returns an `Err` on failure; callers should treat this as fail-closed
/// (delete the file rather than leave it world-readable).
#[cfg(windows)]
pub fn restrict_owner_only(path: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{LocalFree, HLOCAL};
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR,
    };

    // SDDL: Protected DACL, grants Full Access to SYSTEM, Administrators, and Owner.
    const SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;OW)";

    let mut psd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            SDDL.encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
                .as_ptr(),
            SDDL_REVISION_1,
            &mut psd,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 || psd.is_null() {
        return Err(std::io::Error::last_os_error());
    }

    // Build null-terminated wide string path for Win32.
    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let flags = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
    let ok = unsafe { SetFileSecurityW(path_wide.as_ptr(), flags, psd) };

    // Free the security descriptor regardless of outcome.
    unsafe {
        LocalFree(psd as HLOCAL);
    }

    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
