//! `herdr-inbox app`, the real binary. The plan's rule: `answer` never types, asserted against a
//! herdr that fails on any send, with a real sender that does type in the same world.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use herdr_inbox::app::feed::group;
use herdr_inbox::app::feed::text::Text;
use herdr_inbox::store::hosts::CoreSeat;
use herdr_inbox::store::questions::NewQuestion;
use serde_json::{Value, json};

use crate::rig::{BINARY, Rig, exe, wait_for};
use crate::world::World;

const SESSION: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const EMPTY: &str = include_str!("../fixtures/composer/empty.ansi");

/// A herdr server that records every request, and on `on_probe` runs once at the verb's look.
struct Herdr {
    socket: PathBuf,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Herdr {
    fn start(dir: &Path, transcript: PathBuf, on_probe: impl FnMut() + Send + 'static) -> Herdr {
        let socket = dir.join("h.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let mut on_probe = Some(on_probe);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_err() {
                    continue;
                }
                let Ok(request) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let method = request["method"].as_str().unwrap_or_default().to_string();
                log.lock().unwrap().push(method.clone());
                let result = match method.as_str() {
                    "workspace.list" => {
                        if let Some(mut probe) = on_probe.take() {
                            probe();
                        }
                        json!({"workspaces": []})
                    }
                    "agent.get" => json!({"agent": {"pane_id": "w1:p1", "agent": "claude",
                        "agent_status": "idle", "focused": false,
                        "agent_session": {"kind": "id", "value": SESSION}}}),
                    "pane.read" => json!({"read": {"text": EMPTY}}),
                    "agent.prompt" => {
                        let text = request["params"]["text"].as_str().unwrap_or_default();
                        let record =
                            json!({"type": "user", "message": {"role": "user", "content": text}});
                        let mut f = fs::OpenOptions::new()
                            .append(true)
                            .open(&transcript)
                            .unwrap();
                        writeln!(f, "{record}").unwrap();
                        json!({})
                    }
                    _ => json!({}),
                };
                let _ = writeln!(stream, "{}", json!({"id": request["id"], "result": result}));
            }
        });
        Herdr { socket, seen }
    }

    fn count(&self, method: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|m| *m == method)
            .count()
    }
}

struct Live {
    rig: Rig,
    world: World,
    transcript: PathBuf,
}

impl Live {
    fn new() -> Live {
        let rig = Rig::new();
        let world = World::new();
        let project = rig.dir.path().join("claude/projects/-r");
        fs::create_dir_all(&project).unwrap();
        let transcript = project.join(format!("{SESSION}.jsonl"));
        fs::write(&transcript, "{\"type\": \"user\"}\n").unwrap();
        Live {
            rig,
            world,
            transcript,
        }
    }

    /// An open question in pane `w1:p1`, asked on `server`.
    fn question(&self, server: Option<&Path>) -> (i64, String) {
        let q = self
            .world
            .store
            .insert(&NewQuestion {
                question: "Question Text 345",
                pane_id: Some("w1:p1"),
                session_id: Some(SESSION),
                cwd: Some("/r"),
                ..NewQuestion::default()
            })
            .unwrap();
        let server = server.map(|s| s.to_string_lossy().into_owned());
        self.world
            .store
            .set_server(q.id, server.as_deref())
            .unwrap();
        let open = self.world.store.group(q.id, false).unwrap();
        let row = group::entry(&self.world.store, q.id, &open, Text::Cut).unwrap();
        (q.id, row["token"].as_str().unwrap().to_string())
    }

    /// The real verb, from `exe`.
    fn answer(&self, exe: &Path, id: i64, token: &str) -> Output {
        let id = id.to_string();
        self.rig
            .command(exe, &self.world.db)
            .args([
                "app",
                "answer",
                "--entry",
                &format!("group:{id}"),
                "--token",
                token,
            ])
            .args(["--question", &id, "--json", "--detail"])
            .arg(r#"{"choices":[],"notes":{},"text":"Answer Text 567"}"#)
            .output()
            .unwrap()
    }

    fn delivered(&self, id: i64) -> bool {
        self.world
            .store
            .get(id)
            .unwrap()
            .is_some_and(|q| q.delivered_at.is_some())
    }
}

fn reply(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|_| panic!("{out:?}"))
}

/// The world in which a sender does type: without this the next test proves nothing.
#[test]
fn the_sender_the_verb_starts_delivers_on_the_questions_own_server() {
    let live = Live::new();
    let herdr = Herdr::start(live.rig.dir.path(), live.transcript.clone(), || {});
    let (id, token) = live.question(Some(&herdr.socket));
    // the verb's own default socket is dead: only the stored server can reach this agent
    let out = live.answer(Path::new(BINARY), id, &token);
    assert_eq!(reply(&out)["delivery"], "queued", "{out:?}");
    assert!(
        wait_for(Duration::from_secs(20), || live.delivered(id)),
        "{:?}",
        herdr.seen.lock().unwrap()
    );
    assert_eq!(herdr.count("agent.prompt"), 1);
}

/// The sender's binary goes the moment the verb looks at the server, so this verb has no sender:
/// whatever reaches this herdr afterwards can only be the verb's own process.
#[test]
fn answer_never_types_from_its_own_process() {
    let live = Live::new();
    let bin = live.rig.dir.path().join("herdr-inbox");
    exe::link_or_copy(Path::new(BINARY), &bin);
    let gone = bin.clone();
    let herdr = Herdr::start(live.rig.dir.path(), live.transcript.clone(), move || {
        fs::remove_file(&gone).unwrap();
    });
    let (id, token) = live.question(Some(&herdr.socket));
    let out = live.answer(&bin, id, &token);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        reply(&out)["delivery"],
        "queued",
        "a sender that cannot start leaves it to the worker"
    );
    assert_eq!(herdr.count("workspace.list"), 1, "the verb did look");
    assert_eq!(herdr.count("agent.prompt"), 0, "the verb typed");
    assert_eq!(herdr.count("agent.get"), 0, "the verb looked at the agent");
    assert!(!bin.exists());
    assert!(
        live.rig.calls("herdr").is_empty(),
        "{}",
        live.rig.calls("herdr")
    );
    assert!(live.rig.calls("hc").is_empty(), "{}", live.rig.calls("hc"));
    assert_eq!(
        live.world.store.get(id).unwrap().unwrap().status,
        "answered"
    );
    assert!(!live.delivered(id));
}

#[test]
fn a_server_proven_gone_records_the_answer_and_holds_it() {
    let live = Live::new();
    let dead = live.rig.dir.path().join("gone.sock");
    let (id, token) = live.question(Some(&dead));
    let out = live.answer(Path::new(BINARY), id, &token);
    assert_eq!(reply(&out)["delivery"], "held", "{out:?}");
    assert_eq!(
        live.world.store.get(id).unwrap().unwrap().status,
        "answered"
    );
    std::thread::sleep(Duration::from_millis(300));
    let q = live.world.store.get(id).unwrap().unwrap();
    assert_eq!(q.delivery_attempts, 0, "no sender, no attempt used up");
    assert_eq!(q.delivery_error, None);
}

/// Only a refused or missing socket proves a server gone. One that answers nonsense does not.
#[test]
fn a_server_that_answers_badly_proves_nothing_and_the_sender_goes() {
    let live = Live::new();
    let path = live.rig.dir.path().join("odd.sock");
    let listener = UnixListener::bind(&path).unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let _ = writeln!(stream, "not json");
        }
    });
    let (id, token) = live.question(Some(&path));
    let out = live.answer(Path::new(BINARY), id, &token);
    assert_eq!(reply(&out)["delivery"], "queued", "{out:?}");
}

/// Until core delivery lands the real binary starts nothing for a core row, and asks `hc` nothing.
#[test]
fn a_core_answer_is_recorded_and_held() {
    let live = Live::new();
    let (id, token) = live.question(None);
    let store = &live.world.store;
    store
        .db()
        .execute("UPDATE questions SET pane_id = NULL WHERE id = ?", [id])
        .unwrap();
    store.set_server(id, Some("herdr-core")).unwrap();
    let seat = CoreSeat {
        terminal: "t3".to_string(),
        machine: None,
    };
    store.set_question_host(id, &seat).unwrap();
    let open = store.group(id, false).unwrap();
    let moved = group::entry(store, id, &open, Text::Cut).unwrap();
    assert_ne!(moved["token"], token, "the asker is in the token");
    let out = live.answer(Path::new(BINARY), id, moved["token"].as_str().unwrap());
    assert_eq!(reply(&out)["delivery"], "held", "{out:?}");
    assert_eq!(store.get(id).unwrap().unwrap().status, "answered");
    assert!(live.rig.calls("hc").is_empty());
}

#[test]
fn a_stale_token_exits_1_and_starts_nothing() {
    let live = Live::new();
    let herdr = Herdr::start(live.rig.dir.path(), live.transcript.clone(), || {});
    let (id, _) = live.question(Some(&herdr.socket));
    let out = live.answer(Path::new(BINARY), id, &"0".repeat(32));
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(reply(&out)["code"], "stale");
    assert_eq!(live.world.store.get(id).unwrap().unwrap().status, "open");
    std::thread::sleep(Duration::from_millis(300));
    assert!(herdr.seen.lock().unwrap().is_empty());
}

#[test]
fn help_touches_nothing_and_a_bad_word_is_exit_2_with_empty_stdout() {
    let rig = Rig::new();
    let db = rig.dir.path().join("inbox.db");
    let run = |args: &[&str]| {
        rig.command(Path::new(BINARY), &db)
            .args(args)
            .output()
            .unwrap()
    };
    let help = run(&["app", "--help"]);
    assert_eq!(help.status.code(), Some(0));
    assert!(help.stdout.starts_with(b"herdr-inbox app v1\n"));
    let bad = run(&[
        "app", "ack", "--entry", "alert:05", "--token", "t", "--json",
    ]);
    assert_eq!(bad.status.code(), Some(2));
    assert!(bad.stdout.is_empty());
    assert!(!bad.stderr.is_empty());
    assert!(!db.exists());
}

#[test]
fn a_database_nobody_migrated_yet_says_so() {
    let rig = Rig::new();
    let db = rig.dir.path().join("state/herdr-inbox/inbox.db");
    let out = rig
        .command(Path::new(BINARY), &db)
        .args(["app", "ack", "--entry", "alert:5", "--token", "t", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(reply(&out)["code"], "not_migrated");
    assert!(!db.exists(), "a verb never creates the database");
}
