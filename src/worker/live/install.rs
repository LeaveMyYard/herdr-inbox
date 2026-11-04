//! Which install this worker is, and where the inbox is installed now
//! (Python `herdr._own_checkout`, `follows_listed_root`,
//! `current_plugin_root`, `worker.installed_version`).

use std::fs;
use std::path::{Path, PathBuf};

/// The version in `root`'s `pyproject.toml`: its first line starting
/// `version = `, between the first pair of `"` (Python `installed_version`
/// and `running_version`). Equal to the binary's own version until cutover
/// (the release check holds both equal).
pub fn version_at(root: &Path) -> Option<String> {
    let text = fs::read_to_string(root.join("pyproject.toml")).ok()?;
    let line = text.lines().find(|l| l.starts_with("version = "))?;
    line.split('"').nth(1).map(String::from)
}

/// The version of the code this process runs (Python `running_version`):
/// its checkout's `pyproject.toml`, read once at start, else the compiled
/// version. Never the compiled version alone: in a linked checkout pulled
/// but not rebuilt, `target/release/herdr-inbox` still carries the old one,
/// so a worker comparing it with the disk would hand over to that same
/// binary on every tick, forever. Read from disk, the successor sees the
/// version it compared against, and stays.
pub fn running_version(own: Option<&Path>) -> String {
    own.and_then(version_at)
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

/// The checkout holding `exe`: its nearest ancestor with a
/// `herdr-plugin.toml` (a linked checkout's `target/release/herdr-inbox`, a
/// release's `bin/herdr-inbox`). Python also asks for `src/herdr_inbox`,
/// which goes at cutover; the manifest is what makes it the plugin's root.
pub fn own_checkout(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .skip(1)
        .find(|dir| dir.join("herdr-plugin.toml").is_file())
        .map(Path::to_path_buf)
}

/// This worker's install: its own checkout, and whether it follows the root
/// herdr lists (decided once, on herdr's first answer).
#[derive(Clone, Debug)]
pub struct Install {
    own: Option<PathBuf>,
    follows: Option<bool>,
}

impl Install {
    /// The install whose checkout is `own` (`None`: no checkout, an
    /// installed copy).
    pub fn new(own: Option<PathBuf>) -> Self {
        Self { own, follows: None }
    }

    /// This code's checkout.
    pub fn own(&self) -> Option<&Path> {
        self.own.as_deref()
    }

    /// Whether this is herdr's own install, so it follows the root herdr
    /// lists: true with no checkout, or when the checkout was the listed
    /// root the first time herdr listed one. A dev worktree or a Claude-cache
    /// copy whose server lists another checkout never follows it (no
    /// handover to other code, no downgrade). Until herdr has answered: no.
    pub fn follows(&mut self, listed: impl FnOnce() -> Option<PathBuf>) -> bool {
        let Some(own) = &self.own else {
            return true;
        };
        if let Some(decided) = self.follows {
            return decided;
        }
        let Some(listed) = listed() else {
            return false;
        };
        let decided = same_path(&listed, own);
        self.follows = Some(decided);
        decided
    }

    /// Where the inbox is installed now, asked anew each time: herdr's
    /// listed root when this install follows it (herdr 0.9.4+ builds every
    /// update in a new directory), else, or with no answer, this checkout.
    pub fn current(&mut self, mut listed: impl FnMut() -> Option<PathBuf>) -> Option<PathBuf> {
        if self.follows(&mut listed) {
            return listed().or_else(|| self.own.clone());
        }
        self.own.clone()
    }
}

/// Python `os.path.realpath(a) == os.path.realpath(b)`: a path that cannot
/// be resolved compares as written.
fn same_path(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    real(a) == real(b)
}
