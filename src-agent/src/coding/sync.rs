//! A poisoned lock stops normal operations. Cleanup may still need the owned
//! process handles to terminate children and release connections.
use std::sync::{Mutex, MutexGuard};

pub(super) trait CheckedMutex<T> {
    fn checked_lock(&self) -> anyhow::Result<MutexGuard<'_, T>>;
    fn cleanup_lock(&self) -> MutexGuard<'_, T>;
}

impl<T> CheckedMutex<T> for Mutex<T> {
    fn checked_lock(&self) -> anyhow::Result<MutexGuard<'_, T>> {
        self.lock().map_err(|_| {
            anyhow::anyhow!(
                "Coding state was poisoned by a worker panic; restart the coding service"
            )
        })
    }

    fn cleanup_lock(&self) -> MutexGuard<'_, T> {
        match self.lock() {
            Ok(guard) => guard,
            Err(error) => {
                eprintln!("Cleaning up poisoned coding state; normal operations remain disabled");
                error.into_inner()
            }
        }
    }
}
