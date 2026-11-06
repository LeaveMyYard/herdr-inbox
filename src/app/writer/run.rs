//! The writer's life: takes `writer.lock`, says it is ready, and publishes
//! until `control.json`, `stop.request`, a changed binary or an absent app ends
//! it. `feed.json` stays after it exits, and readers see it age.

use std::fs::{self, Metadata};
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use super::control;
use super::ensure::READY;
use super::files::Paths;
use super::idle::{self, Idle};
use super::record::{Record, sha256};
use super::source::Source;

/// How often the writer looks at the database, `control.json` and `stop.request`.
pub const POLL: Duration = Duration::from_millis(250);
/// The writer publishes at least this often, so that `generated_at` moves.
pub const PUBLISH_EVERY: Duration = Duration::from_secs(5);

/// Runs a writer of `exe`, reading `db`, until something ends it. The exit code.
pub fn serve(paths: &Paths, exe: &Path, db: &Path) -> i32 {
    match start(paths, exe) {
        Ok(lock) => {
            run(paths, exe, db);
            drop(lock);
            0
        }
        Err(_) => 1,
    }
}

/// Everything before the first read: the record, then the word to the parent.
fn start(paths: &Paths, exe: &Path) -> Result<fs::File, String> {
    paths.check_folder()?;
    control::enabled(paths)?;
    let lock = paths.lock_file("writer.lock").map_err(|e| e.to_string())?;
    lock.try_lock().map_err(|e| e.to_string())?;
    let hash = sha256(exe).map_err(|e| e.to_string())?;
    Record::write(paths, &hash).map_err(|e| e.to_string())?;
    // a parent that stopped waiting has closed the pipe: no failure of the writer's
    let mut out = std::io::stdout();
    let _ = out.write_all(READY).and_then(|()| out.flush());
    Ok(lock)
}

fn run(paths: &Paths, exe: &Path, db: &Path) {
    let binary = fingerprint(exe);
    let born = Instant::now();
    let mut source = Source::new(db);
    let mut idle = Idle::default();
    let mut asked: Option<Instant> = None;
    let mut published: Option<Instant> = None;
    loop {
        if told_to_stop(paths) {
            return;
        }
        if asked.is_none_or(|at| at.elapsed() >= idle::EVERY) {
            asked = Some(Instant::now());
            if idle.observe(idle::probe(&paths.home), born.elapsed()) {
                return;
            }
        }
        let due = published.is_none_or(|at| at.elapsed() >= PUBLISH_EVERY);
        if source.changed() || due {
            let bytes = source.feed(SystemTime::now());
            // a failed write leaves the last feed to age; the next pass tries again
            let _ = paths.replace("feed.json", ".feed.tmp", &bytes);
            published = Some(Instant::now());
            if fingerprint(exe) != binary {
                return;
            }
        }
        std::thread::sleep(POLL);
    }
}

fn told_to_stop(paths: &Paths) -> bool {
    control::enabled(paths).is_err() || paths.file("stop.request").exists()
}

/// What changes when the binary is rebuilt or replaced.
fn fingerprint(exe: &Path) -> Option<(u64, u64, u64, Option<SystemTime>)> {
    let meta: Metadata = fs::metadata(exe).ok()?;
    Some((meta.dev(), meta.ino(), meta.len(), meta.modified().ok()))
}
