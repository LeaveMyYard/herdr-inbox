//! One watcher per session (Python `rewake._claim_ownership`, `_still_owner`):
//! the newest watcher's token in `<rewake dir>/<session>.owner`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::token::hex8;

/// This watcher's claim on its session's watch.
#[derive(Debug)]
pub struct Owner {
    path: PathBuf,
    token: String,
}

impl Owner {
    /// Writes a fresh token to `path` (a temporary file renamed into place, so
    /// a reader sees a whole token): from now on an older watcher sees it and
    /// exits.
    pub fn claim(path: &Path) -> io::Result<Owner> {
        let token = format!("{}{}{}{}", hex8(), hex8(), hex8(), hex8());
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension(format!("{token}.tmp"));
        fs::write(&tmp, &token)?;
        fs::rename(&tmp, path)?;
        Ok(Owner {
            path: path.to_path_buf(),
            token,
        })
    }

    /// Is this still the newest watcher? An unreadable file says no.
    pub fn still_owner(&self) -> bool {
        fs::read_to_string(&self.path).is_ok_and(|t| t == self.token)
    }
}
