//! Narrow WebKitGTK buffer-transport fallback for issue #254.
//!
//! Kept free of GTK dependencies so sysfs detection and override handling can
//! also be tested on hosts without a Linux desktop.
use std::ffi::OsStr;
use std::path::Path;

#[cfg(all(target_os = "linux", feature = "gui"))]
pub(super) fn configure() {
    const FORCE_SHM: &str = "WEBKIT_DMABUF_RENDERER_FORCE_SHM";
    const DISABLE_DMABUF: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
    // WebKit caches its transport choice on first use. Set this before the
    // event loop initializes GTK, so every subsequently created webview uses it.
    if should_force_shm(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os(FORCE_SHM).as_deref(),
        std::env::var_os(DISABLE_DMABUF).as_deref(),
        Path::new("/sys/class/drm"),
    ) {
        std::env::set_var(FORCE_SHM, "1");
    }
}

fn should_force_shm(
    wayland_display: Option<&OsStr>,
    force_shm: Option<&OsStr>,
    disable_dmabuf: Option<&OsStr>,
    drm_root: &Path,
) -> bool {
    wayland_display.is_some_and(|display| !display.is_empty())
        && force_shm.is_none()
        && disable_dmabuf.is_none()
        && has_ivy_bridge(drm_root)
}

fn has_ivy_bridge(drm_root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(drm_root) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(index) = name.to_str().and_then(|name| name.strip_prefix("card")) else {
            return false;
        };
        // Ignore connector entries (card0-HDMI-A-1) and render nodes. Card
        // numbers are not stable; inspect all primary DRM devices.
        if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        let device = entry.path().join("device");
        let vendor = read_hex(&device.join("vendor"));
        let id = read_hex(&device.join("device"));
        vendor == Some(0x8086)
            && matches!(
                id,
                Some(0x0152 | 0x0156 | 0x015a | 0x0162 | 0x0166 | 0x016a)
            )
    })
}

fn read_hex(path: &Path) -> Option<u16> {
    let value = std::fs::read_to_string(path).ok()?;
    let value = value.trim();
    u16::from_str_radix(value.strip_prefix("0x").unwrap_or(value), 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct DrmFixture(PathBuf);

    impl DrmFixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "koma-drm-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn device(&self, name: &str, vendor: &str, id: &str) {
            let path = self.0.join(name).join("device");
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("vendor"), vendor).unwrap();
            std::fs::write(path.join("device"), id).unwrap();
        }
    }

    impl Drop for DrmFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn ivy_bridge_uses_shared_memory_on_wayland() {
        for id in ["0x0152\n", "0x0156", "0x015a", "0x0162", "0x0166", "0x016a"] {
            let drm = DrmFixture::new();
            drm.device("card1", "0x8086\n", id);
            assert!(should_force_shm(
                Some(OsStr::new("wayland-1")),
                None,
                None,
                &drm.0
            ));
        }
    }

    #[test]
    fn explicit_webkit_overrides_are_preserved_including_zero_and_empty() {
        let drm = DrmFixture::new();
        drm.device("card0", "0x8086", "0x0152");
        for value in ["0", "1", ""] {
            assert!(!should_force_shm(
                Some(OsStr::new("wayland-0")),
                Some(OsStr::new(value)),
                None,
                &drm.0
            ));
            assert!(!should_force_shm(
                Some(OsStr::new("wayland-0")),
                None,
                Some(OsStr::new(value)),
                &drm.0
            ));
        }
    }

    #[test]
    fn x11_and_empty_wayland_display_are_unchanged() {
        let drm = DrmFixture::new();
        drm.device("card0", "0x8086", "0x0152");
        for display in [None, Some(OsStr::new(""))] {
            assert!(!should_force_shm(display, None, None, &drm.0));
        }
    }

    #[test]
    fn other_gpus_and_connector_entries_are_unchanged() {
        let drm = DrmFixture::new();
        drm.device("card0", "0x8086", "0x1912");
        drm.device("card1", "0x1002", "0x0152");
        drm.device("card2-HDMI-A-1", "0x8086", "0x0152");
        drm.device("renderD128", "0x8086", "0x0152");
        assert!(!has_ivy_bridge(&drm.0));
        drm.device("card12", "0x8086", "0x0162");
        assert!(has_ivy_bridge(&drm.0));
    }

    #[test]
    fn missing_or_malformed_sysfs_is_nonfatal() {
        let drm = DrmFixture::new();
        assert!(!has_ivy_bridge(&drm.0.join("missing")));
        drm.device("card0", "not a vendor", "0x0152");
        drm.device("card1", "0x8086", "not a device");
        std::fs::create_dir(drm.0.join("card2")).unwrap();
        assert!(!has_ivy_bridge(&drm.0));
    }
}
