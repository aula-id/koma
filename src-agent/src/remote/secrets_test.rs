#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use std::sync::Mutex;

/// Serialise tests that touch the real secrets dir under a temp HOME override.
static LOCK: Mutex<()> = Mutex::new(());

struct TempHome {
    dir: PathBuf,
    prev_home: Option<std::ffi::OsString>,
    #[cfg(windows)]
    prev_local: Option<std::ffi::OsString>,
}

impl TempHome {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "koma-secrets-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", &dir);
        #[cfg(windows)]
        let prev_local = {
            let prev = std::env::var_os("LOCALAPPDATA");
            std::env::set_var("LOCALAPPDATA", &dir);
            prev
        };
        let _ = crate::model::store::base_dir().map(|p| {
            let _ = std::fs::create_dir_all(&p);
            p
        });
        Self {
            dir,
            prev_home,
            #[cfg(windows)]
            prev_local,
        }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        match &self.prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        #[cfg(windows)]
        match &self.prev_local {
            Some(v) => std::env::set_var("LOCALAPPDATA", v),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn roundtrip_password() {
    let _g = LOCK.lock().unwrap();
    let _home = TempHome::new();
    assert!(get_remote_password("h1").is_none());
    set_remote_password("h1", "s3cret").unwrap();
    assert_eq!(get_remote_password("h1").as_deref(), Some("s3cret"));
}

#[test]
fn wrong_machine_key_returns_none() {
    let _g = LOCK.lock().unwrap();
    let _home = TempHome::new();
    set_remote_password("h1", "s3cret").unwrap();
    // Overwrite machine key with different bytes.
    let path = machine_key_path().unwrap();
    let mut other = [0u8; KEY_LEN];
    rand::thread_rng().fill_bytes(&mut other);
    other[0] ^= 0xff;
    std::fs::write(&path, other).unwrap();
    assert!(get_remote_password("h1").is_none());
}

#[test]
fn delete_removes_entry() {
    let _g = LOCK.lock().unwrap();
    let _home = TempHome::new();
    set_remote_password("h1", "s3cret").unwrap();
    delete_remote_password("h1").unwrap();
    assert!(get_remote_password("h1").is_none());
}

#[test]
fn aad_binds_to_host_id() {
    let _g = LOCK.lock().unwrap();
    let _home = TempHome::new();
    let key = read_or_create_machine_key().unwrap();
    let entry = encrypt(&key, "host-a", "pw").unwrap();
    assert!(decrypt(&key, "host-b", &entry).is_none());
    assert_eq!(decrypt(&key, "host-a", &entry).as_deref(), Some("pw"));
}

#[test]
fn missing_store_is_none() {
    let _g = LOCK.lock().unwrap();
    let _home = TempHome::new();
    assert!(get_remote_password("nope").is_none());
}
