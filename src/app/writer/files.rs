//! The writer's files under `<appHome>/data/herdr-inbox`, its locks and its
//! atomic writes. A file staged by a crash has a name the installer cleans up.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The app home's folder for this mod's data.
pub const DATA: &str = "data/herdr-inbox";
/// How often a wait for a lock looks again.
const LOCK_POLL: Duration = Duration::from_millis(25);

/// The paths of one app home.
#[derive(Clone, Debug)]
pub struct Paths {
    /// `<appHome>`.
    pub home: PathBuf,
    /// `<appHome>/data/herdr-inbox`.
    pub data: PathBuf,
}

impl Paths {
    /// The paths under app home `home`.
    pub fn new(home: &Path) -> Paths {
        Paths {
            home: home.to_path_buf(),
            data: home.join(DATA),
        }
    }

    /// A file of the folder.
    pub fn file(&self, name: &str) -> PathBuf {
        self.data.join(name)
    }

    /// The folder is a real one of this user's, closed to everyone else: the
    /// feed holds question text.
    pub fn check_folder(&self) -> Result<(), String> {
        let meta = fs::symlink_metadata(&self.data).map_err(|e| format!("no app data: {e}"))?;
        let own = rustix::process::geteuid().as_raw();
        if !meta.is_dir() || meta.uid() != own || meta.permissions().mode() & 0o077 != 0 {
            return Err("the app data folder is not a private folder of this user".to_string());
        }
        Ok(())
    }

    /// `name`, opened for its lock, empty and created if it must be.
    pub fn lock_file(&self, name: &str) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .mode(0o600)
            .open(self.file(name))
    }

    /// Replaces `name` with `bytes`, whole or not at all, by way of `staged`.
    pub fn replace(&self, name: &str, staged: &str, bytes: &[u8]) -> io::Result<()> {
        let tmp = self.file(staged);
        match fs::remove_file(&tmp) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, self.file(name))?;
        File::open(&self.data)?.sync_all()
    }

    /// Removes `name` if it is there.
    pub fn remove(&self, name: &str) -> io::Result<()> {
        match fs::remove_file(self.file(name)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// Takes `file`'s `flock` within `wait`; false if another holder keeps it.
pub fn lock_within(file: &File, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    loop {
        if file.try_lock().is_ok() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(LOCK_POLL);
    }
}

#[cfg(test)]
mod tests;
