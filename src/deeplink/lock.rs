//! The running popup's lock (Python `deeplink.popup_running`,
//! `popup_is_running`, `hold_popup_lock`): `<state>/popup.lock`, held
//! exclusively for the popup's lifetime.

use std::path::{Path, PathBuf};

use herdr_agent_kit::plugin_env::lock::{HeldLock, LockFile};

/// The lock file in the inbox's state directory.
pub fn lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join("popup.lock")
}

/// A popup holds the lock (probed without waiting, shared, released at once).
/// A file that cannot be opened reads as no popup.
pub fn popup_is_running(state_dir: &Path) -> bool {
    LockFile::new(lock_path(state_dir)).held_elsewhere()
}

/// Takes the lock for this popup's lifetime (drop it to release). `None`
/// when another popup holds it (or it cannot be taken): this popup runs all
/// the same, as in Python.
pub fn hold_popup_lock(state_dir: &Path) -> Option<HeldLock> {
    std::fs::create_dir_all(state_dir).ok()?;
    LockFile::new(lock_path(state_dir))
        .try_exclusive()
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests;
