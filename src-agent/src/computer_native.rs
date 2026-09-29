//! Raw ABI for the GUI-owned desktop worker. Ownership, approvals and operation
//! serialization are enforced by the executable before calling this interface.
use std::ffi::c_char;

unsafe extern "C" {
    /// Execute a serialized request against the platform desktop SDK.
    ///
    /// # Safety
    /// `request` must point to a live, NUL-terminated UTF-8 JSON string. Calls
    /// that access controller state must be serialized by the desktop worker.
    /// Free a non-null returned allocation exactly once with `koma_computer_free`.
    pub fn koma_computer_call(request: *const c_char) -> *mut c_char;

    /// Release a native response allocation.
    ///
    /// # Safety
    /// `reply` must be null or an unfreed pointer returned by `koma_computer_call`.
    /// It must not be accessed after this call.
    pub fn koma_computer_free(reply: *mut c_char);

    /// Set the native cancellation flag; no desktop SDK operations run here.
    ///
    /// # Safety
    /// The caller must own the current controller generation. This may run
    /// concurrently with its worker's request, but not cancel a later controller.
    pub fn koma_computer_cancel();

    /// Set the Dock and menu-bar icon from a PNG or icns file.
    ///
    /// # Safety
    /// `path` must be null or a live, NUL-terminated UTF-8 filesystem path.
    /// The call must run on the main thread after `NSApplication` exists.
    pub fn koma_set_app_icon(path: *const c_char);

    /// Bring this process forward as a regular foreground app.
    ///
    /// # Safety
    /// Must run on the main thread after `NSApplication` exists.
    #[cfg(target_os = "macos")]
    pub fn koma_activate_app();
}
