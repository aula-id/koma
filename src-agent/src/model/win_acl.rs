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
    use windows_sys::Win32::Security::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW,
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        SetNamedSecurityInfoW, SE_FILE_OBJECT, SDDL_REVISION_1,
    };

    // SDDL: Protected DACL, grants Full Access to SYSTEM, Administrators, and Owner.
    let sddl: &[u16] = &[
        b'D' as u16, b':' as u16, b'P' as u16, b'(' as u16,
        b'A' as u16, b';' as u16, b';' as u16, b'F' as u16, b'A' as u16, b';' as u16,
        b';' as u16, b';' as u16, b'S' as u16, b'Y' as u16, b')' as u16,
        b'(' as u16, b'A' as u16, b';' as u16, b';' as u16, b'F' as u16, b'A' as u16,
        b';' as u16, b';' as u16, b';' as u16, b'B' as u16, b'A' as u16, b')' as u16,
        b'(' as u16, b'A' as u16, b';' as u16, b';' as u16, b'F' as u16, b'A' as u16,
        b';' as u16, b';' as u16, b';' as u16, b'O' as u16, b'W' as u16, b')' as u16,
        0,
    ];

    let mut psd: *mut std::ffi::c_void = std::ptr::null_mut();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut psd,
            std::ptr::null(),
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
    let result = unsafe {
        SetNamedSecurityInfoW(
            path_wide.as_ptr(),
            SE_FILE_OBJECT,
            flags,
            std::ptr::null(),
            std::ptr::null(),
            psd,
            std::ptr::null(),
        )
    };

    // Free the security descriptor regardless of outcome.
    unsafe {
        LocalFree(psd as HLOCAL);
    }

    if result != 0 {
        return Err(std::io::Error::from_raw_os_error(result as i32));
    }
    Ok(())
}
