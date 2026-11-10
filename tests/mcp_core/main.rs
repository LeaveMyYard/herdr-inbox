//! `ask_operator` from a herdr-core terminal, with real processes: a stand-in
//! `claude` (this executable, by that name) above the real `herdr-inbox mcp`,
//! and an `hc` that prints what it is told. So the walk up the process table,
//! the start token and the bounded `hc` reads are all the real ones.
//!
//! This executable is its own test runner (`harness = false`). Run as
//! `claude CMD ARGS...` it runs CMD as a child and waits, as Claude Code runs
//! an MCP server. Every process has an environment built here from nothing,
//! and the stand-in is orphaned at start: a run of this suite from an agent's
//! tool must not put a real Claude above it, which would read as nesting.

#[path = "../support/exe.rs"]
mod exe;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use herdr_agent_kit::agent_procs::start_token;
use herdr_agent_kit::herdr_api::fake::{FakeHerdr, Reply};
use herdr_inbox::mcp::receipts::{SHARED, UNDECIDED, UNPROVED};
use herdr_inbox::store::Store;
use herdr_inbox::store::hosts::{CORE_SERVER, CoreSeat};
use herdr_inbox::store::questions::NewQuestion;
use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-inbox");
const SESSION: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const OTHER: &str = "bbbbbbbb-0000-4000-8000-00000000000b";

/// Starts `$0 "$@"` in the background and writes its pid. Through fd 3, because dash gives a
/// background job /dev/null as stdin first, so `<&0` would read that.
const ORPHAN: &str = "exec 3<&0; \"$0\" \"$@\" <&3 3<&- & echo $! > stand-in.pid";

type Case = (&'static str, fn());

const CASES: &[Case] = &[
    (
        "a_proved_agent_files_a_question",
        a_proved_agent_files_a_question,
    ),
    (
        "a_proved_agent_raises_an_alert",
        a_proved_agent_raises_an_alert,
    ),
    (
        "a_proved_agent_fetches_its_answer",
        a_proved_agent_fetches_its_answer,
    ),
    (
        "a_spoofed_terminal_is_unavailable",
        a_spoofed_terminal_is_unavailable,
    ),
    (
        "a_nested_claude_is_unavailable",
        a_nested_claude_is_unavailable,
    ),
    ("no_hc_is_unavailable", no_hc_is_unavailable),
    (
        "a_session_with_no_claim_never_runs_hc",
        a_session_with_no_claim_never_runs_hc,
    ),
    (
        "with_no_core_running_hc_is_never_run",
        with_no_core_running_hc_is_never_run,
    ),
    (
        "a_process_between_claude_and_the_server_is_refused",
        a_process_between_claude_and_the_server_is_refused,
    ),
    (
        "a_slow_first_second_never_takes_an_inherited_pane",
        a_slow_first_second_never_takes_an_inherited_pane,
    ),
    (
        "a_hanging_hc_never_takes_an_inherited_pane",
        a_hanging_hc_never_takes_an_inherited_pane,
    ),
    (
        "check_answers_cannot_read_the_inherited_panes_answer",
        check_answers_cannot_read_the_inherited_panes_answer,
    ),
    (
        "a_pane_that_holds_this_claude_stays_a_pane",
        a_pane_that_holds_this_claude_stays_a_pane,
    ),
    (
        "claude_codes_record_names_the_session",
        claude_codes_record_names_the_session,
    ),
    (
        "an_unusable_record_is_unavailable",
        an_unusable_record_is_unavailable,
    ),
    (
        "hc_never_gets_the_callers_pane",
        hc_never_gets_the_callers_pane,
    ),
    ("a_core_at_hc_socket_is_found", a_core_at_hc_socket_is_found),
    (
        "a_proved_terminals_hook_carries_its_answer",
        a_proved_terminals_hook_carries_its_answer,
    ),
    (
        "a_hook_with_a_copied_claim_carries_nothing",
        a_hook_with_a_copied_claim_carries_nothing,
    ),
    (
        "a_hook_with_no_host_never_runs_hc",
        a_hook_with_no_host_never_runs_hc,
    ),
    (
        "a_twins_hook_carries_nothing_until_one_copy_ends",
        a_twins_hook_carries_nothing_until_one_copy_ends,
    ),
    (
        "a_refused_claim_is_no_pane_for_a_hook",
        a_refused_claim_is_no_pane_for_a_hook,
    ),
    (
        "a_watcher_in_a_proved_terminal_wakes_it_at_rest",
        a_watcher_in_a_proved_terminal_wakes_it_at_rest,
    ),
    (
        "a_watcher_never_wakes_a_blocked_terminal",
        a_watcher_never_wakes_a_blocked_terminal,
    ),
    (
        "a_resume_in_a_proved_terminal_is_marked_with_the_terminal",
        a_resume_in_a_proved_terminal_is_marked_with_the_terminal,
    ),
    (
        "a_watcher_goes_by_the_terminal_before_inherited_pane_variables",
        a_watcher_goes_by_the_terminal_before_inherited_pane_variables,
    ),
    (
        "the_core_worker_types_through_hc",
        the_core_worker_types_through_hc,
    ),
    (
        "a_core_terminals_server_starts_the_core_worker",
        a_core_terminals_server_starts_the_core_worker,
    ),
    (
        "a_filed_question_says_where_its_records_are",
        a_filed_question_says_where_its_records_are,
    ),
    (
        "two_live_copies_of_a_session_are_refused_until_one_ends",
        two_live_copies_of_a_session_are_refused_until_one_ends,
    ),
    (
        "a_stale_record_refuses_nothing",
        a_stale_record_refuses_nothing,
    ),
    (
        "a_record_mid_rewrite_says_to_call_again",
        a_record_mid_rewrite_says_to_call_again,
    ),
];

/// Runs under the stand-in, as Claude Code runs a hook. `hc get` reports `$1` as the terminal's
/// foreground, or the stand-in (`$PPID`) when that is empty. The server comes after, only so
/// that [`Rig::start`] finds one.
const HOOK: &str = r#"
fg=${1:-$PPID}
printf '{"address":"t3","state":"live","status":"%s","foreground":{"pid":%s,"name":"claude","argv":["%s","sh","hook.sh"],"cwd":"/f"}}' \
  "${STATUS:-done}" "$fg" "$CLAUDE" > hc.get
if [ -n "$WATCH" ]; then
  ("$BIN" hook rewake-watch < payload > hook.out ; echo $? > hook.code) &
else
  "$BIN" hook "${EVENT:-stop}" < payload > hook.out
fi
exec "$BIN" mcp
"#;

/// One stand-in session.
struct Rig {
    dir: tempfile::TempDir,
    claude: PathBuf,
    /// Listens where a core's socket would be, so that `hc` may run. Never a real core: every
    /// process here has `XDG_RUNTIME_DIR` under this rig.
    core: Option<UnixListener>,
}

/// A running `claude` with its MCP server under it.
struct Session {
    pid: u32,
    stdin: ChildStdin,
    replies: mpsc::Receiver<Value>,
    next: i64,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("claude");
        exe::link_or_copy(&std::env::current_exe().unwrap(), &claude);
        let db = dir.path().join("inbox.db");
        Store::open_migrating(&db, Duration::from_secs(5)).unwrap();
        exe::install(
            &dir.path().join("herdr-cli"),
            "#!/bin/sh\necho \"$@\" >> \"$0.log\"\nexit 1\n",
        );
        let socket = dir.path().join("run/herdr-core/default.sock");
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        let core = Some(UnixListener::bind(socket).unwrap());
        Rig { dir, claude, core }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// An `hc` that answers from `hc.get` and `hc.ls` beside it.
    fn install_hc(&self) {
        exe::install(
            &self.path("hc"),
            "#!/bin/sh\necho \"$@ pane=[$HERDR_PANE_ID]\" >> \"$0.log\"\n\
             case \"$1\" in get) cat \"$0.get\";; ls) cat \"$0.ls\";; read) cat \"$0.read\";; send) echo '{}';; *) exit 2;; esac\n",
        );
    }

    /// `claude <argv>`, with `HC_TERMINAL_ID` set to `claim`.
    fn start(&self, claim: Option<&str>, argv: &[&str]) -> Session {
        self.start_with(claim, argv, &[])
    }

    /// [`Rig::start`] with more variables.
    fn start_with(&self, claim: Option<&str>, argv: &[&str], extra: &[(&str, String)]) -> Session {
        let dir = self.dir.path();
        let mut cmd = Command::new("sh");
        cmd.env_clear()
            .args(["-c", ORPHAN])
            .arg(&self.claude)
            .args(argv)
            .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
            .env("HOME", dir)
            .env("XDG_STATE_HOME", dir)
            .env("XDG_CONFIG_HOME", dir)
            .env("XDG_RUNTIME_DIR", dir.join("run"))
            .env("CLAUDE_CONFIG_DIR", dir.join("claude-config"))
            .env("CLAUDE_CODE_SESSION_ID", SESSION)
            .env("HERDR_INBOX_DB", dir.join("inbox.db"))
            .env("HERDR_INBOX_NO_WORKER", "1")
            .env("HERDR_BIN_PATH", dir.join("herdr-cli"))
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(claim) = claim {
            cmd.env("HC_TERMINAL_ID", claim);
        }
        for (name, value) in extra {
            cmd.env(name, value);
        }
        if extra.iter().any(|(k, _)| *k == "WORKERS") {
            cmd.env_remove("HERDR_INBOX_NO_WORKER");
        }
        let mut starter = cmd.spawn().unwrap();
        let stdin = starter.stdin.take().unwrap();
        let out = starter.stdout.take().unwrap();
        assert!(starter.wait().unwrap().success());
        let pid = std::fs::read_to_string(self.path("stand-in.pid")).unwrap();
        let pid = pid.trim().parse().unwrap();
        let (tx, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(serde_json::from_str::<Value>(&line).unwrap());
            }
        });
        let mut s = Session {
            pid,
            stdin,
            replies,
            next: 1,
        };
        s.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "stand-in", "version": "0"}}),
        );
        s.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        s
    }

    /// `hc` says terminal `t3`'s foreground is `pid`, and `argv` its command line.
    fn hc_reports(&self, pid: u32, argv: &[&str]) {
        self.install_hc();
        let get = json!({"address": "t3", "state": "live", "foreground":
            {"pid": pid, "name": "claude", "argv": argv, "cwd": "/fixture"}});
        let ls = json!({"hidden_finished": 0, "terminals":
            [{"address": "t3", "agent": "claude", "cwd": "/fixture", "state": "live", "turn": 0}]});
        std::fs::write(self.path("hc.get"), get.to_string()).unwrap();
        std::fs::write(self.path("hc.ls"), ls.to_string()).unwrap();
    }

    /// A session in t3 whose foreground hc reports truly.
    fn proved(&self) -> Session {
        let s = self.start(Some("t3"), &[BIN, "mcp"]);
        let claude = self.claude.display().to_string();
        self.hc_reports(s.pid, &[&claude, BIN, "mcp"]);
        s
    }

    fn db(&self) -> Store {
        Store::open(&self.path("inbox.db"), Duration::from_secs(5)).unwrap()
    }

    /// An answered core question of [`SESSION`], and a Stop hook of that session run in
    /// `claim`, with `foreground` as what `hc` reports. What the hook printed, and the answer's id.
    fn hook(&self, claim: Option<&str>, foreground: &str) -> (String, i64) {
        self.hook_with(claim, foreground, &[])
    }

    /// [`Rig::hook`] with more variables.
    fn hook_with(
        &self,
        claim: Option<&str>,
        foreground: &str,
        more: &[(&str, String)],
    ) -> (String, i64) {
        let store = self.db();
        let q = store
            .insert(&NewQuestion {
                session_id: Some(SESSION),
                question: "Merge it?",
                ..NewQuestion::default()
            })
            .unwrap();
        store.set_server(q.id, Some(CORE_SERVER)).unwrap();
        let seat = CoreSeat {
            terminal: "t3".into(),
            machine: None,
        };
        store.set_question_host(q.id, &seat).unwrap();
        store.set_answer(q.id, "yes", None).unwrap();
        self.install_hc();
        std::fs::write(self.path("hook.sh"), HOOK).unwrap();
        let source = more.iter().find(|(k, _)| *k == "SOURCE").map(|(_, v)| v);
        let payload = json!({"session_id": SESSION, "hook_event_name": "Stop", "source": source});
        std::fs::write(self.path("payload"), payload.to_string()).unwrap();
        let mut extra = vec![
            ("BIN", BIN.to_string()),
            ("CLAUDE", self.claude.display().to_string()),
        ];
        extra.extend_from_slice(more);
        let argv = ["sh", "hook.sh", foreground];
        let s = self.start_with(claim, &argv, &extra);
        if more.iter().any(|(k, _)| *k == "WATCH") {
            // the watcher reads the record of this Claude, known only now
            self.record(s.pid, &|r| r["sessionId"] = json!(SESSION));
            for _ in 0..200 {
                if self.path("hook.code").exists() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        let printed = std::fs::read_to_string(self.path("hook.out")).unwrap_or_default();
        (printed, q.id)
    }

    fn count(&self, table: &str) -> i64 {
        self.db()
            .db()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }
}

impl Session {
    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let m = self.replies.recv_timeout(Duration::from_secs(20)).unwrap();
            if m["id"] == id {
                return m;
            }
        }
    }

    /// The structured result of tool `name`.
    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let r = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        r["result"]["structuredContent"].clone()
    }

    fn ask(&mut self) -> Value {
        self.tool("ask_operator", json!({"question": "Question Title 123?"}))
    }
}

fn a_proved_agent_files_a_question() {
    let rig = Rig::new();
    let mut s = rig.proved();
    let receipt = s.ask();
    assert_eq!(receipt["status"], "open", "{receipt}");
    let id = receipt["id"].as_i64().unwrap();
    let store = rig.db();
    let q = store.get(id).unwrap().unwrap();
    assert_eq!(q.pane_id, None, "`t3` must never read as a pane");
    assert_eq!(q.tab_id, None);
    assert_eq!(q.session_id.as_deref(), Some(SESSION));
    assert_eq!(store.server_of(id).unwrap().as_deref(), Some("herdr-core"));
    let seat: (String, String, Option<String>) = store
        .db()
        .query_row(
            "SELECT host, terminal, machine FROM question_host WHERE id = ?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(seat, ("herdr-core".into(), "t3".into(), None));
    let again = s.ask();
    assert_eq!(again["status"], "already_open", "{again}");
    assert_eq!(rig.count("question_host"), 1);
    assert!(
        !rig.path("herdr-cli.log").exists(),
        "herdr is not this host"
    );
}

fn a_proved_agent_raises_an_alert() {
    let rig = Rig::new();
    let mut s = rig.proved();
    let receipt = s.tool(
        "alert_operator",
        json!({"title": "Please check Project Alpha"}),
    );
    assert_eq!(receipt["status"], "open", "{receipt}");
    let id = receipt["id"].as_i64().unwrap();
    let store = rig.db();
    let a = store.alert(id).unwrap().unwrap();
    assert_eq!(a.pane_id, None);
    assert_eq!(a.herdr_server.as_deref(), Some("herdr-core"));
    assert_eq!(rig.count("alert_host"), 1);
    assert!(!rig.path("herdr-cli.log").exists());
    let gone = s.tool("withdraw_alert", json!({"id": id}));
    assert_eq!(gone["status"], "withdrawn", "{gone}");
}

fn a_proved_agent_fetches_its_answer() {
    let rig = Rig::new();
    let mut s = rig.proved();
    let id = s.ask()["id"].as_i64().unwrap();
    rig.db()
        .db()
        .execute(
            "UPDATE questions SET status = 'answered', answer = 'Yes', \
             answered_at = '2026-11-06T10:00:00+00:00' WHERE id = ?",
            [id],
        )
        .unwrap();
    let got = s.tool("check_answers", json!({}));
    assert_eq!(got["answers"][0]["answer"], "Yes", "{got}");
}

fn a_spoofed_terminal_is_unavailable() {
    // HC_TERMINAL_ID says t3, and t3's foreground is some other Claude
    let rig = Rig::new();
    let mut s = rig.start(Some("t3"), &[BIN, "mcp"]);
    let claude = rig.claude.display().to_string();
    rig.hc_reports(s.pid + 1, &[&claude, BIN, "mcp"]);
    let receipt = s.ask();
    assert_eq!(receipt["status"], "unavailable", "{receipt}");
    assert_eq!(rig.count("questions"), 0);
    let receipt = s.tool("alert_operator", json!({"title": "Title"}));
    assert_eq!(receipt["status"], "unavailable", "{receipt}");
    assert_eq!(rig.count("alerts"), 0);
}

fn a_nested_claude_is_unavailable() {
    // the outer claude really is t3's foreground; the server runs under an inner one
    let rig = Rig::new();
    let claude = rig.claude.display().to_string();
    let mut s = rig.start(Some("t3"), &[&claude, BIN, "mcp"]);
    rig.hc_reports(s.pid, &[&claude, &claude, BIN, "mcp"]);
    let receipt = s.ask();
    assert_eq!(receipt["status"], "unavailable", "{receipt}");
    assert_eq!(
        receipt["message"], UNPROVED,
        "a nested agent must not wait for this"
    );
    assert_eq!(rig.count("questions"), 0);
}

fn no_hc_is_unavailable() {
    let rig = Rig::new();
    let mut s = rig.start(Some("t3"), &[BIN, "mcp"]);
    let receipt = s.ask();
    assert_eq!(receipt["status"], "unavailable", "{receipt}");
}

fn a_session_with_no_claim_never_runs_hc() {
    let rig = Rig::new();
    rig.install_hc();
    let mut s = rig.start(None, &[BIN, "mcp"]);
    let receipt = s.ask();
    assert_eq!(receipt["status"], "unavailable", "{receipt}");
    assert!(!rig.path("hc.log").exists());
}

fn with_no_core_running_hc_is_never_run() {
    // any hc command would start a core, and the core would keep this process's variables
    let mut rig = Rig::new();
    rig.core = None;
    let mut s = rig.start(Some("t3"), &[BIN, "mcp"]);
    let claude = rig.claude.display().to_string();
    rig.hc_reports(s.pid, &[&claude, BIN, "mcp"]);
    let receipt = s.ask();
    assert_eq!(receipt["status"], "unavailable", "{receipt}");
    assert!(!rig.path("hc.log").exists());
}

fn a_process_between_claude_and_the_server_is_refused() {
    // an unrecognised harness or a tool: any descendant in the terminal is not its agent
    let rig = Rig::new();
    let script = format!("{BIN} mcp; true");
    let mut s = rig.start(Some("t3"), &["/bin/sh", "-c", &script]);
    let claude = rig.claude.display().to_string();
    rig.hc_reports(s.pid, &[&claude, "/bin/sh", "-c", &script]);
    let receipt = s.ask();
    assert_eq!(receipt["status"], "unavailable", "{receipt}");
    assert_eq!(rig.count("questions"), 0);
}

/// `pane.process_info` with `foreground` the pane's one foreground process.
fn foreground_is(foreground: u32) -> Reply {
    Reply::Result(json!({"process_info": {"pane_id": "w9:p9",
        "foreground_processes": [{"pid": foreground, "name": "claude", "argv": ["claude"]}]},
        "type": "process_info"}))
}

/// herdr with pane w9:p9, whose agent runs `OTHER` and whose foreground is `foreground`.
fn other_pane(foreground: u32) -> FakeHerdr {
    let f = FakeHerdr::start();
    let pane = json!({"pane_id": "w9:p9", "tab_id": "w9:t9", "workspace_id": "w9", "agent": "claude",
        "agent_session": {"agent": "claude", "kind": "id", "source": "herdr:claude", "value": OTHER}});
    let tab = json!({"tab_id": "w9:t9", "label": "other-tab", "workspace_id": "w9"});
    f.on(
        "pane.get",
        Reply::Result(json!({"pane": pane, "type": "pane_info"})),
    )
    .on(
        "tab.get",
        Reply::Result(json!({"tab": tab, "type": "tab_info"})),
    )
    .on("pane.process_info", foreground_is(foreground));
    f
}

/// What a core started from that pane hands to every terminal.
fn inherited(f: &FakeHerdr) -> Vec<(&'static str, String)> {
    vec![
        ("HERDR_PANE_ID", "w9:p9".to_owned()),
        (
            "HERDR_SOCKET_PATH",
            f.socket().to_string_lossy().into_owned(),
        ),
    ]
}

impl Rig {
    /// The terminal's first second: live, and hc has no foreground for it yet.
    fn hc_has_no_foreground(&self) {
        self.install_hc();
        let get = json!({"address": "t3", "state": "live", "foreground": null});
        std::fs::write(self.path("hc.get"), get.to_string()).unwrap();
    }

    /// `hc` past its budget.
    fn hc_hangs(&self) {
        exe::install(&self.path("hc"), "#!/bin/sh\nexec sleep 30\n");
    }

    /// Claude Code's record of process `pid`, with `change` applied.
    fn record(&self, pid: u32, change: &dyn Fn(&mut Value)) {
        let dir = self.path("claude-config/sessions");
        std::fs::create_dir_all(&dir).unwrap();
        let mut record = json!({"pid": pid, "sessionId": OTHER, "kind": "interactive",
            "procStart": start_token(pid).unwrap(), "cwd": "/fixture", "startedAt": 0});
        change(&mut record);
        std::fs::write(dir.join(format!("{pid}.json")), record.to_string()).unwrap();
    }
}

fn a_slow_first_second_never_takes_an_inherited_pane() {
    // the review's Q6
    let rig = Rig::new();
    let f = other_pane(1234);
    rig.hc_has_no_foreground();
    let mut s = rig.start_with(Some("t3"), &[BIN, "mcp"], &inherited(&f));
    let receipt = s.ask();
    assert_eq!(receipt["status"], "unavailable", "{receipt}");
    assert_eq!(rig.count("questions"), 0);
}

fn a_hanging_hc_never_takes_an_inherited_pane() {
    // the review's Q8
    let rig = Rig::new();
    let f = other_pane(1234);
    rig.hc_hangs();
    let mut s = rig.start_with(Some("t3"), &[BIN, "mcp"], &inherited(&f));
    let receipt = s.ask();
    assert_eq!(receipt["status"], "unavailable", "{receipt}");
    assert_eq!(rig.count("questions"), 0);
}

fn check_answers_cannot_read_the_inherited_panes_answer() {
    let rig = Rig::new();
    let f = other_pane(1234);
    rig.hc_hangs();
    let store = rig.db();
    store
        .db()
        .execute(
            "INSERT INTO questions (created_at, pane_id, session_id, question, norm, status, answer, \
             answered_at) VALUES ('2026-11-06T10:00:00+00:00', 'w9:p9', ?, 'Question Title 123?', \
             'question title 123', 'answered', 'Yes', '2026-11-06T10:01:00+00:00')",
            [OTHER],
        )
        .unwrap();
    let mut s = rig.start_with(Some("t3"), &[BIN, "mcp"], &inherited(&f));
    let got = s.tool("check_answers", json!({}));
    assert_eq!(got["status"], "unavailable", "{got}");
    assert_eq!(store.get(1).unwrap().unwrap().acknowledged_at, None);
    assert_eq!(
        got["message"], UNDECIDED,
        "an answer waits: the agent must call again"
    );
}

fn a_pane_that_holds_this_claude_stays_a_pane() {
    // a herdr TUI started inside a core terminal: herdr itself lists this Claude
    let rig = Rig::new();
    rig.hc_has_no_foreground();
    let f = other_pane(1);
    let mut s = rig.start_with(Some("t3"), &[BIN, "mcp"], &inherited(&f));
    f.on("pane.process_info", foreground_is(s.pid));
    let receipt = s.ask();
    assert_eq!(receipt["status"], "open", "{receipt}");
    let q = rig.db().get(1).unwrap().unwrap();
    assert_eq!(q.pane_id.as_deref(), Some("w9:p9"));
    assert_eq!(q.session_id.as_deref(), Some(OTHER));
    assert_eq!(rig.count("question_host"), 0);
}

fn claude_codes_record_names_the_session() {
    // after `/clear` the environment still names the session the process started with
    let rig = Rig::new();
    let mut s = rig.proved();
    rig.record(s.pid, &|_| {});
    assert_eq!(s.ask()["status"], "open");
    let q = rig.db().get(1).unwrap().unwrap();
    assert_eq!(q.session_id.as_deref(), Some(OTHER));
}

fn an_unusable_record_is_unavailable() {
    let rig = Rig::new();
    let mut s = rig.proved();
    rig.record(s.pid, &|r| r["kind"] = json!("background"));
    let receipt = s.ask();
    assert_eq!(receipt["status"], "unavailable", "{receipt}");
    assert_eq!(rig.count("questions"), 0);
}

fn hc_never_gets_the_callers_pane() {
    // a core that hc started in a race would hand these to every terminal
    let rig = Rig::new();
    let f = other_pane(1);
    let mut s = rig.start_with(Some("t3"), &[BIN, "mcp"], &inherited(&f));
    let claude = rig.claude.display().to_string();
    rig.hc_reports(s.pid, &[&claude, BIN, "mcp"]);
    assert_eq!(s.ask()["status"], "open");
    let log = std::fs::read_to_string(rig.path("hc.log")).unwrap();
    assert!(log.contains("pane=[]") && !log.contains("w9:p9"), "{log}");
}

fn a_core_at_hc_socket_is_found() {
    let mut rig = Rig::new();
    rig.core = None;
    let socket = rig.path("elsewhere.sock");
    let _core = UnixListener::bind(&socket).unwrap();
    let extra = [("HC_SOCKET", socket.to_string_lossy().into_owned())];
    let mut s = rig.start_with(Some("t3"), &[BIN, "mcp"], &extra);
    let claude = rig.claude.display().to_string();
    rig.hc_reports(s.pid, &[&claude, BIN, "mcp"]);
    assert_eq!(s.ask()["status"], "open");
}

/// Waits until process `pid` is gone.
fn gone(pid: u32) {
    for _ in 0..500 {
        if start_token(pid).is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("{pid} is still there");
}

fn two_live_copies_of_a_session_are_refused_until_one_ends() {
    // the final pass: `claude --resume <id>` of a session still live in another terminal
    let rig = Rig::new();
    let mut mine = rig.proved();
    rig.record(mine.pid, &|_| {});
    let copy = rig.start(Some("t4"), &[BIN, "mcp"]);
    rig.record(copy.pid, &|_| {});
    let store = rig.db();
    store
        .db()
        .execute(
            "INSERT INTO questions (created_at, session_id, question, norm, status, answer, \
             answered_at) VALUES ('2026-11-06T10:00:00+00:00', ?, 'Question Title 123?', \
             'question title 123', 'answered', 'Yes', '2026-11-06T10:01:00+00:00')",
            [OTHER],
        )
        .unwrap();
    let got = s_tool(&mut mine, "check_answers");
    assert_eq!(got["message"], SHARED, "{got}");
    assert_eq!(store.get(1).unwrap().unwrap().acknowledged_at, None);
    assert_eq!(mine.ask()["message"], SHARED);
    assert_eq!(rig.count("questions"), 1);
    let pid = copy.pid;
    drop(copy); // its server sees EOF and it ends; its record stays, as after a crash
    gone(pid);
    let got = s_tool(&mut mine, "check_answers");
    assert_eq!(got["answers"][0]["answer"], "Yes", "{got}");
}

fn s_tool(s: &mut Session, name: &str) -> Value {
    s.tool(name, json!({}))
}

fn a_stale_record_refuses_nothing() {
    // the pid of a record whose process is gone, and a live pid with another start token
    let rig = Rig::new();
    let mut s = rig.proved();
    rig.record(s.pid, &|_| {});
    let dir = rig.path("claude-config/sessions");
    let dead = json!({"pid": 999_999, "sessionId": OTHER, "kind": "interactive", "procStart": "1"});
    std::fs::write(dir.join("999999.json"), dead.to_string()).unwrap();
    let copy = rig.start(None, &[BIN, "mcp"]);
    rig.record(copy.pid, &|r| r["procStart"] = json!("an earlier process"));
    assert_eq!(s.ask()["status"], "open");
}

fn a_record_mid_rewrite_says_to_call_again() {
    let rig = Rig::new();
    let mut s = rig.proved();
    rig.record(s.pid, &|_| {});
    let copy = rig.start(None, &[BIN, "mcp"]);
    let file = rig.path(&format!("claude-config/sessions/{}.json", copy.pid));
    std::fs::write(&file, "").unwrap();
    assert_eq!(s.ask()["message"], UNDECIDED);
    rig.record(copy.pid, &|r| r["sessionId"] = json!(SESSION));
    assert_eq!(s.ask()["status"], "open");
}

/// Run as `claude`: CMD as a child, as Claude Code runs an MCP server.
fn stand_in(argv: &[String]) -> ExitCode {
    let done = Command::new(&argv[0]).args(&argv[1..]).status();
    ExitCode::from(u8::from(!done.is_ok_and(|s| s.success())))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if Path::new(&args[0])
        .file_name()
        .is_some_and(|n| n == "claude")
    {
        return stand_in(&args[1..]);
    }
    let wanted: Vec<&String> = args[1..].iter().filter(|a| !a.starts_with('-')).collect();
    let mut failed = 0;
    for (name, case) in CASES {
        if !wanted.is_empty() && !wanted.iter().any(|w| name.contains(w.as_str())) {
            continue;
        }
        let ok = catch_unwind(AssertUnwindSafe(case)).is_ok();
        println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        failed += usize::from(!ok);
    }
    println!(
        "test result: {}. {failed} failed",
        if failed == 0 { "ok" } else { "FAILED" }
    );
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn a_proved_terminals_hook_carries_its_answer() {
    let rig = Rig::new();
    let (printed, id) = rig.hook(Some("t3"), "");
    assert!(printed.contains("[inbox]"), "{printed}");
    assert!(printed.contains(&format!("Q{id}")), "{printed}");
    let q = rig.db().get(id).unwrap().unwrap();
    assert!(q.delivered_at.is_some(), "recorded before it was printed");
}

fn a_hook_with_a_copied_claim_carries_nothing() {
    let rig = Rig::new();
    // another terminal's claim: its foreground is somebody else
    let (printed, id) = rig.hook(Some("t3"), "4242");
    assert_eq!(printed, "");
    assert_eq!(rig.db().get(id).unwrap().unwrap().delivered_at, None);
}

fn a_hook_with_no_host_never_runs_hc() {
    let rig = Rig::new();
    let (printed, id) = rig.hook(None, "");
    assert_eq!(printed, "");
    assert_eq!(rig.db().get(id).unwrap().unwrap().delivered_at, None);
    let log = std::fs::read_to_string(rig.path("hc.log")).unwrap_or_default();
    assert_eq!(log, "", "no claim, no hc");
}

fn a_twins_hook_carries_nothing_until_one_copy_ends() {
    // the review of #146, H3: `claude --resume <id>` of a live session in another terminal
    let rig = Rig::new();
    rig.install_hc();
    let copy = rig.start(Some("t4"), &[BIN, "mcp"]);
    rig.record(copy.pid, &|r| r["sessionId"] = json!(SESSION));
    let (printed, id) = rig.hook(Some("t3"), "");
    assert_eq!(printed, "", "one copy would take the other's answer");
    assert_eq!(rig.db().get(id).unwrap().unwrap().delivered_at, None);
    let pid = copy.pid;
    drop(copy);
    gone(pid);
    let (printed, _) = rig.hook(Some("t3"), "");
    assert!(printed.contains(&format!("Q{id}")), "{printed}");
}

fn a_refused_claim_is_no_pane_for_a_hook() {
    // a core hands its environment on: the pane variables prove nothing beside a claim
    let rig = Rig::new();
    let pane = [("HERDR_PANE_ID", "w1:p1".to_string())];
    let (printed, id) = rig.hook_with(Some("t3"), "4242", &pane);
    assert_eq!(printed, "");
    assert_eq!(rig.db().get(id).unwrap().unwrap().delivered_at, None);
}

fn a_filed_question_says_where_its_records_are() {
    // a core worker of another config directory finds them through this
    let rig = Rig::new();
    let dir = rig.path("claude-config/projects/-r");
    std::fs::create_dir_all(&dir).unwrap();
    let transcript = dir.join(format!("{SESSION}.jsonl"));
    std::fs::write(&transcript, "{}\n").unwrap();
    let mut s = rig.proved();
    assert_eq!(s.ask()["status"], "open");
    let known: String = rig
        .db()
        .db()
        .query_row(
            "SELECT transcript_path FROM sessions WHERE session_id = ?",
            [SESSION],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(Path::new(&known), transcript);
}

fn a_watcher_in_a_proved_terminal_wakes_it_at_rest() {
    let rig = Rig::new();
    let (printed, id) = rig.hook_with(Some("t3"), "", &[("WATCH", "1".to_string())]);
    assert!(printed.contains(&format!("Q{id}")), "{printed:?}");
    let code = std::fs::read_to_string(rig.path("hook.code")).unwrap();
    assert_eq!(code.trim(), "3", "the wake exit");
}

fn a_watcher_never_wakes_a_blocked_terminal() {
    let rig = Rig::new();
    let more = [
        ("WATCH", "1".to_string()),
        ("STATUS", "blocked".to_string()),
    ];
    let (printed, id) = rig.hook_with(Some("t3"), "", &more);
    assert_eq!(printed, "");
    assert_eq!(rig.db().get(id).unwrap().unwrap().delivered_at, None);
}

fn a_resume_in_a_proved_terminal_is_marked_with_the_terminal() {
    let rig = Rig::new();
    let more = [
        ("EVENT", "session-start".to_string()),
        ("SOURCE", "resume".to_string()),
        ("HERDR_PANE_ID", "w1:p1".to_string()), // inherited: the terminal goes first
    ];
    rig.hook_with(Some("t3"), "", &more);
    let marker = ["starts", "herdr-inbox/starts"]
        .iter()
        .map(|d| rig.path(d).join(SESSION))
        .find(|p| p.exists())
        .expect("no marker");
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "t3");
}

fn a_watcher_goes_by_the_terminal_before_inherited_pane_variables() {
    let rig = Rig::new();
    let more = [
        ("WATCH", "1".to_string()),
        ("HERDR_PANE_ID", "w1:p1".to_string()),
    ];
    let (printed, id) = rig.hook_with(Some("t3"), "", &more);
    assert!(printed.contains(&format!("Q{id}")), "{printed:?}");
}

/// The real worker, keyed on the core, over a real proof of a stand-in Claude.
fn the_core_worker_types_through_hc() {
    let rig = Rig::new();
    let s = rig.proved();
    rig.record(s.pid, &|r| r["sessionId"] = json!(SESSION));
    let box_ = "\u{2500}".repeat(20);
    let read =
        json!({"complete": true, "how": "screen", "text": format!("{box_}\n\u{276f} \n{box_}\n")});
    std::fs::write(rig.path("hc.read"), read.to_string()).unwrap();
    let get = std::fs::read_to_string(rig.path("hc.get")).unwrap();
    let mut get: Value = serde_json::from_str(&get).unwrap();
    get["status"] = json!({"app": "claude-code", "state": "idle"});
    std::fs::write(rig.path("hc.get"), get.to_string()).unwrap();
    let store = rig.db();
    let q = store
        .insert(&NewQuestion {
            session_id: Some(SESSION),
            question: "Merge it?",
            ..NewQuestion::default()
        })
        .unwrap();
    store.set_server(q.id, Some(CORE_SERVER)).unwrap();
    let seat = CoreSeat {
        terminal: "t3".into(),
        machine: None,
    };
    store.set_question_host(q.id, &seat).unwrap();
    store.set_answer(q.id, "yes", None).unwrap();
    store
        .db()
        .execute_batch("UPDATE questions SET answered_at = '2020-01-01T00:00:00+00:00'")
        .unwrap();
    let dir = rig.dir.path();
    let mut worker = Command::new(BIN)
        .arg("worker")
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
        .env("HOME", dir)
        .env("XDG_STATE_HOME", dir)
        .env("XDG_CONFIG_HOME", dir)
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .env("CLAUDE_CONFIG_DIR", dir.join("claude-config"))
        .env("HERDR_INBOX_DB", dir.join("inbox.db"))
        .env("HERDR_BIN_PATH", dir.join("herdr-cli"))
        .env("HERDR_SOCKET_PATH", "herdr-core")
        .env("HERDR_INBOX_WORKER_TICK", "0.1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // the outcome, not the send: the worker records it after `hc` returns, and a slow
    // machine (the macOS runner) shows a gap between the two
    let delivered = || rig.db().get(q.id).unwrap().unwrap().delivered_at.is_some();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !delivered() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    // a few more ticks: a second send would show here
    std::thread::sleep(Duration::from_millis(500));
    let _ = worker.kill(); // this test's own child
    let _ = worker.wait();
    let log = std::fs::read_to_string(rig.path("hc.log")).unwrap_or_default();
    let sends = log.lines().filter(|l| l.starts_with("send ")).count();
    assert!(delivered(), "the outcome was never written");
    assert_eq!(sends, 1, "typed once, through hc");
}

fn a_core_terminals_server_starts_the_core_worker() {
    let rig = Rig::new();
    let lock = rig.path(&format!("worker-{}.lock", "0fd11ed85d43"));
    rig.install_hc();
    let more = [
        ("WORKERS", "1".to_string()),
        ("HERDR_INBOX_WORKER_TICK", "0.1".to_string()),
    ];
    let _s = rig.start_with(Some("t3"), &[BIN, "mcp"], &more);
    let mut held = false;
    for _ in 0..100 {
        held = herdr_agent_kit::plugin_env::LockFile::new(&lock).held_elsewhere();
        if held {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(held, "no core worker");
    // detached, so no child of this test: it ends by its own rule, the core gone
}
