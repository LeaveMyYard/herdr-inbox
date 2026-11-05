//! A stand-in for herdr's CLI (`HERDR_BIN_PATH`) in every rig that runs
//! the binary: it logs its arguments and exits 1, so the code under test
//! never reaches the machine's own herdr (which `PATH` could find, #118),
//! and a test can say which CLI calls it expected.

use std::fs;
use std::path::{Path, PathBuf};

#[path = "exe.rs"]
pub mod exe;

/// The stub under `dir` and its log.
pub struct CliStub {
    /// The executable, for `HERDR_BIN_PATH`.
    pub bin: PathBuf,
    log: PathBuf,
}

impl CliStub {
    /// Writes the stub into `dir` (created if missing).
    pub fn new(dir: &Path) -> CliStub {
        fs::create_dir_all(dir).unwrap();
        let bin = dir.join("herdr-cli-stub");
        let log = dir.join("herdr-cli-stub.log");
        exe::install(&bin, "#!/bin/sh\necho \"$*\" >> \"$0.log\"\nexit 1\n");
        CliStub { bin, log }
    }

    /// Every call so far, one line of arguments each.
    pub fn calls(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    /// Panics unless every call so far is one of `expected`.
    pub fn assert_only(&self, expected: &[&str]) {
        let calls = self.calls();
        let unexpected: Vec<&String> = calls
            .iter()
            .filter(|c| !expected.contains(&c.as_str()))
            .collect();
        assert!(
            unexpected.is_empty(),
            "unexpected herdr CLI calls: {unexpected:?}"
        );
    }
}
