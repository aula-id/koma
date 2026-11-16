//! The SDK advisory lock on `config.json`. It orders SDK writers among
//! themselves; daemons, the terminal and the GUI do not take it.
use crate::app::runtime::sdk::protocol::{Reason, RpcError};
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(20);

/// Held while a config write runs; dropping it releases the lock.
#[derive(Debug)]
pub struct ConfigLock {
    _file: File,
}

/// Take the exclusive lock on `path`, polling for up to `wait`. The file is
/// created owner-only and is never removed, so every writer locks the same
/// inode. A symlink or any other non-file at `path` is refused, so the lock
/// path cannot be used to create or lock a file elsewhere.
pub fn acquire(path: &Path, wait: Duration) -> Result<ConfigLock, RpcError> {
    let file = open(path).map_err(|e| {
        RpcError::reason(
            Reason::IoError,
            format!("config lock could not be opened: {:?}", e.kind()),
        )
    })?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(ConfigLock { _file: file }),
            Err(TryLockError::WouldBlock) if started.elapsed() < wait => std::thread::sleep(POLL),
            Err(TryLockError::WouldBlock) => {
                return Err(RpcError::reason(
                    Reason::LockTimeout,
                    format!(
                        "another SDK config write held the lock for {} ms",
                        wait.as_millis()
                    ),
                ))
            }
            Err(TryLockError::Error(e)) => {
                return Err(RpcError::reason(
                    Reason::IoError,
                    format!("config lock failed: {:?}", e.kind()),
                ))
            }
        }
    }
}

fn open(path: &Path) -> std::io::Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.is_file() => return Err(not_a_file()),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A symlink planted after the check above still fails to open.
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: open a reparse point itself, never its
        // target, so the check below sees it and refuses it.
        const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(not_a_file());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // A lock file left with a wider mode is narrowed to owner-only.
        if meta.permissions().mode() & 0o077 != 0 {
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(file)
}

fn not_a_file() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "the lock path is not a regular file",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("koma-lock-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_held_lock_times_out_and_is_free_after_drop() {
        let dir = temp_dir();
        let path = dir.join("config.json.sdk-lock");
        let held = acquire(&path, Duration::ZERO).unwrap();
        let error = acquire(&path, Duration::from_millis(60)).unwrap_err();
        assert_eq!(error.code, "unavailable");
        assert_eq!(error.delivery, "not_sent");
        assert_eq!(error.extra().and_then(|e| e.reason), Some("lock_timeout"));
        drop(held);
        assert!(acquire(&path, Duration::ZERO).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_at_the_lock_path_is_refused_and_its_target_is_not_created() {
        let dir = temp_dir();
        let path = dir.join("config.json.sdk-lock");
        let target = dir.join("elsewhere");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let error = acquire(&path, Duration::ZERO).unwrap_err();
        assert_eq!(error.code, "unavailable");
        assert_eq!(error.extra().and_then(|e| e.reason), Some("io_error"));
        assert!(!error.message.contains("elsewhere"));
        assert!(!target.exists());
        // A symlink to an existing file is refused too, and the file is untouched.
        std::fs::write(&target, b"keep").unwrap();
        assert!(acquire(&path, Duration::ZERO).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        let _ = std::fs::remove_dir_all(dir);
    }
}
