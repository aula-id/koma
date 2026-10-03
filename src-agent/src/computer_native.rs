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

    /// Resident bytes for `pid`, or 0 when it is dead or not Koma/WebKit.
    /// The current process is always measured.
    ///
    /// # Safety
    /// `pid` is a plain integer. Safe to call from any thread.
    #[cfg(target_os = "macos")]
    pub fn koma_resident_size(pid: u32) -> u64;

    /// Physical memory from `hw.memsize`, or 0 on failure.
    ///
    /// # Safety
    /// No pointers. Safe to call from any thread.
    #[cfg(target_os = "macos")]
    pub fn koma_physical_memory() -> u64;

    /// Write up to `cap` direct child pids of `pid` into `out`.
    ///
    /// # Safety
    /// `out` must be writable for `cap` elements. `cap` may be 0 when `out` is null.
    #[cfg(target_os = "macos")]
    pub fn koma_child_pids(pid: u32, out: *mut u32, cap: u32) -> u32;

    /// Create the menu-bar item on first call and repaint it.
    ///
    /// # Safety
    /// `stats` must be a live [`KomaUsageStats`]. Call on the main thread after
    /// `NSApplication` exists.
    #[cfg(target_os = "macos")]
    pub fn koma_usage_bar_update(stats: *const KomaUsageStats);

    /// Replace the menu-bar role rows. `count` is 0..=5. Each pointer is a
    /// UTF-8 C string. The call copies them before returning.
    ///
    /// # Safety
    /// `labels` and `values` must each be readable for `count` pointers, or
    /// null when `count` is 0. Every string must be valid for the call.
    #[cfg(target_os = "macos")]
    pub fn koma_usage_bar_set_roles(
        labels: *const *const std::ffi::c_char,
        values: *const *const std::ffi::c_char,
        count: u32,
    );
}

/// Menu-bar usage snapshot. Field order matches `KomaUsageStats` in `macos.mm`.
#[cfg(target_os = "macos")]
#[repr(C)]
pub struct KomaUsageStats {
    pub mem_window: u64,
    pub mem_agent: u64,
    pub mem_services: u64,
    pub mem_system: u64,
    pub tokens_in: u64,
    pub tokens_cached: u64,
    pub tokens_out: u64,
    pub cost_micros: u64,
    pub context_window: u64,
    pub working: u8,
}
