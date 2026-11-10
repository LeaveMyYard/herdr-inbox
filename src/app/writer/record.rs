//! `writer.json`, the writer's record: diagnostics and busy proof, never a
//! signal target.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::files::Paths;
use super::ledger::now_ms;

const NAME: &str = "writer.json";

/// The record.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Record {
    v: u8,
    /// The writer's process.
    pub pid: u32,
    /// When it started, Unix milliseconds.
    pub start: u64,
    /// The hash of its binary.
    pub binary_sha256: String,
}

impl Record {
    /// The record of `paths`; `None` if there is none that can be read.
    pub fn read(paths: &Paths) -> Option<Record> {
        let bytes = std::fs::read(paths.file(NAME)).ok()?;
        serde_json::from_slice(&bytes)
            .ok()
            .filter(|r: &Record| r.v == 1)
    }

    /// Records this process as the writer of `hash`.
    pub fn write(paths: &Paths, hash: &str) -> io::Result<()> {
        let record = Record {
            v: 1,
            pid: std::process::id(),
            start: now_ms(),
            binary_sha256: hash.to_string(),
        };
        let bytes = serde_json::to_vec(&record)?;
        paths.replace(NAME, ".herdr-inbox-writer.tmp", &bytes)
    }
}

/// The SHA-256 of the file at `path`, in hex.
pub fn sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut chunk = vec![0_u8; 1 << 16];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        hash.update(&chunk[..n]);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
