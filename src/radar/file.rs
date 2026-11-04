//! Reading and replacing herdr's `config.toml` (Python `radar._read`,
//! `_write`): raw bytes (CRLF passes through), an atomic replace of the
//! file a symlink points at, the original kept as `<name>.inbox-bak`, and
//! only if nobody wrote it meanwhile.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// The file's text, if it can be read as UTF-8.
pub fn read(path: &Path) -> Option<String> {
    String::from_utf8(fs::read(path).ok()?).ok()
}

/// Replaces `path`'s target with `after`, if it still holds `before`;
/// `false` when it changed since it was read (or cannot be written).
pub fn write(path: &Path, before: &str, after: &str) -> bool {
    let Ok(path) = fs::canonicalize(path) else {
        return false;
    };
    let unchanged = || fs::read(&path).is_ok_and(|now| now == before.as_bytes());
    if !unchanged() {
        return false; // radar (or the operator) wrote in the meantime: next time
    }
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return false;
    };
    let name = name.to_string_lossy();
    if fs::write(dir.join(format!("{name}.inbox-bak")), before).is_err() {
        return false;
    }
    let tmp = dir.join(format!(".{name}.inbox-{}", std::process::id()));
    let written = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&tmp)
        .and_then(|mut f| f.write_all(after.as_bytes()).and_then(|()| f.sync_all()))
        .and_then(|()| fs::metadata(&path))
        .and_then(|m| {
            fs::set_permissions(
                &tmp,
                fs::Permissions::from_mode(m.permissions().mode() & 0o7777),
            )
        });
    if written.is_err() || !unchanged() {
        let _ = fs::remove_file(&tmp);
        return false;
    }
    fs::rename(&tmp, &path).is_ok()
}
