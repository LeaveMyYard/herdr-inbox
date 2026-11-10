//! `app-feed --ensure` and the writer it starts, as real processes. Every test ends its writer
//! through `stop.request`, the one way a writer is ever stopped.

use std::fs::{self, File};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::rig::{BINARY, Rig, exe, wait_for};
use crate::world::{World, fixture};

/// A writer's rig: stops its writer when it goes.
struct Feed {
    rig: Rig,
    world: World,
    exe: PathBuf,
}

impl Drop for Feed {
    fn drop(&mut self) {
        let _ = File::create(self.rig.data().join("stop.request"));
        wait_for(Duration::from_secs(5), || !self.running());
    }
}

impl Feed {
    fn new() -> Feed {
        let rig = Rig::new();
        rig.install();
        let world = World::of(&fixture("app-feed/every-kind.json"));
        let exe = PathBuf::from(BINARY);
        Feed { rig, world, exe }
    }

    fn ensure(&self) -> Output {
        self.ensure_with(&self.exe)
    }

    fn ensure_with(&self, exe: &Path) -> Output {
        self.rig
            .command(exe, &self.world.db)
            .args(["app-feed", "--ensure", "--app-home"])
            .arg(&self.rig.home)
            .output()
            .unwrap()
    }

    fn running(&self) -> bool {
        let lock = File::open(self.rig.data().join("writer.lock"));
        lock.is_ok_and(|l| l.try_lock().is_err())
    }

    fn feed(&self) -> Option<Value> {
        serde_json::from_slice(&fs::read(self.rig.data().join("feed.json")).ok()?).ok()
    }

    fn record(&self) -> Value {
        serde_json::from_slice(&fs::read(self.rig.data().join("writer.json")).unwrap()).unwrap()
    }

    fn first_feed(&self) -> Value {
        assert!(wait_for(Duration::from_secs(5), || self.feed().is_some()));
        self.feed().unwrap()
    }
}

fn ok(out: &Output) {
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty());
}

#[test]
fn help_starts_nothing_and_touches_no_file() {
    let rig = Rig::new();
    let db = rig.dir.path().join("inbox.db");
    let out = rig
        .command(Path::new(BINARY), &db)
        .args(["app-feed", "--help"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.starts_with(b"herdr-inbox app-feed v1\n"));
    assert!(!rig.home.exists());
    assert!(!db.exists());
}

#[test]
fn bad_arguments_are_exit_2_with_no_io() {
    let rig = Rig::new();
    let db = rig.dir.path().join("inbox.db");
    let out = rig
        .command(Path::new(BINARY), &db)
        .args(["app-feed", "--ensure", "--app-home", "relative"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(!rig.home.exists());
}

#[test]
fn no_install_is_exit_1_with_the_reason_on_stderr() {
    let rig = Rig::new();
    let db = rig.dir.path().join("inbox.db");
    let out = rig
        .command(Path::new(BINARY), &db)
        .args(["app-feed", "--ensure"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(!out.stderr.is_empty());
}

#[test]
fn a_writer_publishes_the_world_and_follows_it() {
    let f = Feed::new();
    ok(&f.ensure());
    let first = f.first_feed();
    let want = fixture("app-feed/every-kind.json");
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(
        first["entries"].as_array().unwrap().len(),
        want["entries"].as_array().unwrap().len()
    );
    let mode = |p: PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(f.rig.data().join("feed.json")), 0o600);
    assert_eq!(mode(f.rig.data()), 0o700);
    assert_eq!(f.record()["v"], 1);

    f.world
        .store
        .db()
        .execute("UPDATE alerts SET status = 'acked' WHERE id = 5", [])
        .unwrap();
    let gone = |feed: Option<Value>| {
        feed.is_some_and(|v| {
            v["entries"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["key"] != "alert:5")
        })
    };
    assert!(
        wait_for(Duration::from_secs(3), || gone(f.feed())),
        "a change shows within a poll"
    );

    let at = f.feed().unwrap()["generated_at"].as_u64().unwrap();
    assert!(
        wait_for(Duration::from_secs(8), || f.feed().unwrap()["generated_at"]
            .as_u64()
            .unwrap()
            > at),
        "generated_at moves with nothing else"
    );
}

#[test]
fn a_reader_never_sees_half_a_file() {
    let f = Feed::new();
    ok(&f.ensure());
    f.first_feed();
    for i in 0..200 {
        let db = f.world.store.db();
        db.execute(
            "UPDATE alerts SET message = ? WHERE id = 5",
            [format!("Question Text {i}")],
        )
        .unwrap();
        let bytes = fs::read(f.rig.data().join("feed.json")).unwrap();
        serde_json::from_slice::<Value>(&bytes).expect("a whole file");
        thread::sleep(Duration::from_millis(5));
    }
    assert!(!f.rig.data().join(".feed.tmp").exists() || f.running());
}

#[test]
fn ensure_is_idempotent_and_two_at_once_start_one_writer() {
    let f = Feed::new();
    let start = || {
        f.rig
            .command(&f.exe, &f.world.db)
            .args(["app-feed", "--ensure"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let both = [start(), start()];
    for child in both {
        ok(&child.wait_with_output().unwrap());
    }
    let pid = f.record()["pid"].clone();
    ok(&f.ensure());
    assert_eq!(f.record()["pid"], pid, "the writer that ran stays");
    assert!(!f.rig.data().join("start.json").exists());
}

#[test]
fn the_writer_reads_only_and_a_missing_database_is_no_empty_inbox() {
    let f = Feed::new();
    let gone = f.rig.dir.path().join("none/inbox.db");
    let out = f
        .rig
        .command(&f.exe, &gone)
        .args(["app-feed", "--ensure"])
        .output()
        .unwrap();
    ok(&out);
    let feed = f.first_feed();
    assert_eq!(feed["ok"], false);
    assert_eq!(feed["entries"].as_array().unwrap().len(), 0);
    assert!(!gone.exists(), "the writer creates no database");
    assert!(f.rig.calls("herdr").is_empty(), "and calls no herdr");
    assert!(f.rig.calls("hc").is_empty(), "and no hc");
}

#[test]
fn stop_request_and_disabled_end_a_writer_within_a_poll_and_the_feed_stays() {
    let f = Feed::new();
    ok(&f.ensure());
    f.first_feed();
    File::create(f.rig.data().join("stop.request")).unwrap();
    assert!(
        wait_for(Duration::from_secs(2), || !f.running()),
        "stop.request"
    );
    assert!(f.feed().is_some());
    fs::remove_file(f.rig.data().join("stop.request")).unwrap();

    ok(&f.ensure());
    fs::write(
        f.rig.data().join("control.json"),
        br#"{"v":1,"disabled":true}"#,
    )
    .unwrap();
    assert!(
        wait_for(Duration::from_secs(2), || !f.running()),
        "disabled"
    );
    assert_eq!(
        f.ensure().status.code(),
        Some(1),
        "a late call starts nothing"
    );
    assert!(!f.running());
}

/// `herdr-app status` says no window: 30 s later the writer has gone, and the next ensure brings
/// one back. Anything else says nothing, which the unit test of `Idle` covers.
#[test]
fn it_idles_out_after_30_seconds_of_a_confirmed_absent_window() {
    let f = Feed::new();
    f.rig.app_status(2);
    ok(&f.ensure());
    f.first_feed();
    thread::sleep(Duration::from_secs(20));
    assert!(f.running(), "not before its time");
    assert!(
        wait_for(Duration::from_secs(20), || !f.running()),
        "it left"
    );
    assert!(f.rig.data().join("control.json").exists());
    assert!(f.feed().is_some());
    f.rig.app_status(0);
    ok(&f.ensure());
    assert!(f.running());
}

#[test]
fn a_writer_leaves_when_its_binary_changes_and_the_next_ensure_replaces_it() {
    let f = Feed::new();
    let bin = f.rig.dir.path().join("herdr-inbox");
    exe::link_or_copy(Path::new(BINARY), &bin);
    ok(&f.ensure_with(&bin));
    f.first_feed();
    // an update: another file at that path
    let staged = f.rig.dir.path().join("herdr-inbox.new");
    fs::copy(BINARY, &staged).unwrap();
    fs::rename(&staged, &bin).unwrap();
    assert!(
        wait_for(Duration::from_secs(8), || !f.running()),
        "it noticed at a publish"
    );
    ok(&f.ensure_with(&bin));
    assert!(f.running());
}

#[test]
fn ensure_with_another_binary_asks_the_running_writer_to_leave() {
    let f = Feed::new();
    ok(&f.ensure());
    let pid = f.record()["pid"].clone();
    // a binary with other bytes: a byte past the end, which no loader or signature check reads
    let other = f.rig.dir.path().join("other-inbox");
    fs::copy(BINARY, &other).unwrap();
    let mut bytes = fs::OpenOptions::new().append(true).open(&other).unwrap();
    std::io::Write::write_all(&mut bytes, b"\0").unwrap();
    drop(bytes);
    ok(&f.ensure_with(&other));
    assert_ne!(f.record()["pid"], pid);
    assert!(f.running());
    assert!(!f.rig.data().join("stop.request").exists());
}
