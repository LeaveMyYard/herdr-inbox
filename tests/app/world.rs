//! A database that holds a feed fixture's world: every row of `tests/fixtures/app-feed/*.json`
//! written as the store writes it. Fake data only.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use herdr_inbox::store::Store;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

/// The server of a question no case says otherwise about.
pub const ALPHA: &str = "/run/herdr-alpha.sock";
/// A session id that stands for "it has a session".
pub const SESSION: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const CORE: &str = "herdr-core";

/// A fixture from `tests/fixtures/`.
pub fn fixture(path: &str) -> Value {
    let full = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(path);
    serde_json::from_str(&std::fs::read_to_string(full).unwrap()).unwrap()
}

/// When a feed fixture was read.
pub fn wall(feed: &Value) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(feed["generated_at"].as_u64().unwrap())
}

/// A migrated database in a directory of its own.
pub struct World {
    /// Kept for its life.
    _dir: tempfile::TempDir,
    /// The database.
    pub db: PathBuf,
    /// A write connection.
    pub store: Store,
}

impl World {
    /// An empty one.
    pub fn new() -> World {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("inbox.db");
        let store = Store::open_migrating(&db, Duration::from_secs(5)).unwrap();
        World {
            _dir: dir,
            db,
            store,
        }
    }

    /// One holding `feed` and its marks.
    pub fn of(feed: &Value) -> World {
        let world = World::new();
        world.seed(feed);
        world
    }

    fn db(&self) -> &Connection {
        self.store.db()
    }

    /// Writes every row of `feed`, then each `answered` row no entry holds.
    pub fn seed(&self, feed: &Value) {
        for e in feed["entries"].as_array().unwrap() {
            self.row(e);
        }
        for a in feed["answered"].as_array().into_iter().flatten() {
            let id = a["question_id"].as_i64().unwrap();
            let filed: i64 = self
                .db()
                .query_row("SELECT COUNT(*) FROM questions WHERE id = ?", [id], |r| {
                    r.get(0)
                })
                .unwrap();
            if filed == 0 {
                self.answered(a);
            }
        }
        self.marks(feed);
    }

    /// An answered question an `answered` row shows: options A and B, the row's picks, and
    /// typed text when it says so.
    pub fn answered(&self, a: &Value) {
        let id = a["question_id"].as_i64().unwrap();
        let group = a["group"].as_i64().unwrap();
        let q =
            json!({"question": "Question Text 345", "asked_by": a["asked_by"], "context": null});
        self.question(
            id,
            group,
            a["created_at"].as_str().unwrap(),
            &a["asker"],
            &q,
        );
        let typed = a["typed"].as_bool().unwrap().then_some("Answer Text 567");
        let detail = json!({"choices": a["choices"], "notes": {}, "text": typed});
        let delivered = a["delivered"]
            .as_bool()
            .unwrap()
            .then(|| a["answered_at"].clone());
        self.db()
            .execute(
                "UPDATE questions SET status = 'answered', answer = 'Answer Text 567', answered_at = ?, \
                 delivered_at = ? WHERE id = ?",
                params![a["answered_at"].as_str(), delivered.as_ref().and_then(Value::as_str), id],
            )
            .unwrap();
        let options = json!([{"label": "Option A"}, {"label": "Option B"}]);
        self.db()
            .execute(
                "UPDATE question_ext SET header = ?, options_json = ?, answer_json = ?, tool_use_id = ? WHERE id = ?",
                params![
                    a["header"].as_str(),
                    options.to_string(),
                    detail.to_string(),
                    a["tool_use_id"].as_str(),
                    id
                ],
            )
            .unwrap();
        if let Some(title) = a["title"].as_str() {
            self.db()
                .execute(
                    "INSERT OR IGNORE INTO question_groups (group_id, title) VALUES (?, ?)",
                    params![group, title],
                )
                .unwrap();
        }
    }

    /// Writes one row of a feed.
    pub fn row(&self, e: &Value) {
        match e["kind"].as_str().unwrap() {
            "group" => self.group(e),
            "failed" => self.failed(e),
            "alert" => self.alert(e),
            "remote_group" => self.remote_group(e),
            "remote_alert" => self.remote_alert(e),
            "remote_outbox" => self.outbox(e),
            "remote_problem" => self.problem(e),
            kind => panic!("no such kind: {kind}"),
        }
    }

    fn group(&self, e: &Value) {
        let group = e["group"].as_i64().unwrap();
        let qs = e["questions"].as_array().unwrap();
        if group < qs[0]["id"].as_i64().unwrap() {
            self.closed(group, group);
        }
        if let Some(title) = e["title"].as_str() {
            self.db()
                .execute(
                    "INSERT INTO question_groups (group_id, title) VALUES (?, ?)",
                    params![group, title],
                )
                .unwrap();
        }
        let tool_use_id = e["tool_use_id"].as_str();
        for q in qs {
            let id = q["id"].as_i64().unwrap();
            self.question(id, group, e["created_at"].as_str().unwrap(), &e["asker"], q);
            let options = q["options"].as_array().unwrap();
            let draft = (!q["draft"].is_null()).then(|| q["draft"].to_string());
            self.db()
                .execute(
                    "UPDATE question_ext SET header = ?, multi_select = ?, options_json = ?, draft_json = ?, \
                     tool_use_id = ? WHERE id = ?",
                    params![
                        q["header"].as_str(),
                        q["multi_select"].as_bool().unwrap(),
                        (!options.is_empty()).then(|| q["options"].to_string()),
                        draft,
                        tool_use_id,
                        id
                    ],
                )
                .unwrap();
        }
    }

    fn question(&self, id: i64, group: i64, at: &str, asker: &Value, q: &Value) {
        self.db()
            .execute(
                "INSERT INTO questions (id, created_at, pane_id, tab_label, session_id, cwd, asked_by, question, \
                 context, norm) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'n')",
                params![
                    id,
                    at,
                    asker["pane"].as_str(),
                    asker["tab_label"].as_str(),
                    SESSION,
                    asker["cwd"].as_str(),
                    q["asked_by"].as_str(),
                    q["question"].as_str().unwrap(),
                    q["context"].as_str()
                ],
            )
            .unwrap();
        let terminal = asker["terminal"].as_str();
        let server = match (terminal, asker["pane"].as_str()) {
            (Some(_), _) => Some(CORE),
            (None, Some(_)) => Some(ALPHA),
            _ => None,
        };
        self.db()
            .execute(
                "INSERT INTO question_ext (id, group_id, herdr_server) VALUES (?, ?, ?)",
                params![id, group, server],
            )
            .unwrap();
        if let Some(t) = terminal {
            self.db()
                .execute(
                    "INSERT INTO question_host (id, host, terminal) VALUES (?, ?, ?)",
                    params![id, CORE, t],
                )
                .unwrap();
        }
    }

    /// A dismissed question nothing shows.
    pub fn closed(&self, id: i64, group: i64) {
        let q = json!({"question": "Question Text 345", "asked_by": null, "context": null});
        let asker = json!({"pane": null, "tab_label": null, "cwd": null, "terminal": null});
        self.question(id, group, "2026-01-02T09:00:00+00:00", &asker, &q);
        self.db()
            .execute(
                "UPDATE questions SET status = 'dismissed' WHERE id = ?",
                [id],
            )
            .unwrap();
    }

    fn failed(&self, e: &Value) {
        let id = e["question_id"].as_i64().unwrap();
        let group = e["group"].as_i64().unwrap();
        self.question(id, group, "2026-01-02T09:30:00+00:00", &e["asker"], e);
        let d = &e["delivery"];
        let reason = d["reason"].as_str().unwrap_or_default();
        let (error, gone) = match d["state"].as_str().unwrap() {
            "unattempted" => (None, None),
            "claimed" => (
                Some("sending:2026-01-02T09:40:00+00:00:abcd1234".to_string()),
                None,
            ),
            "pending" => (Some(format!("pending: {reason}")), None),
            "agent_gone" => (
                Some(format!("agent gone: {reason}")),
                Some("2026-01-02T09:50:00+00:00"),
            ),
            _ => (Some(reason.to_string()), None),
        };
        self.db()
            .execute(
                "UPDATE questions SET status = 'answered', answer = ?, answered_at = ?, delivery_error = ?, \
                 delivery_attempts = ? WHERE id = ?",
                params![
                    e["answer"].as_str(),
                    e["answered_at"].as_str(),
                    error,
                    d["attempts"].as_i64(),
                    id
                ],
            )
            .unwrap();
        let retries = e["retries"].as_i64().filter(|n| *n > 0);
        self.db()
            .execute(
                "UPDATE question_ext SET header = ?, pending_since = ?, agent_gone_at = ?, retry_count = ? WHERE id = ?",
                params![e["header"].as_str(), d["pending_since"].as_str(), gone, retries, id],
            )
            .unwrap();
    }

    fn alert(&self, e: &Value) {
        let id = e["id"].as_i64().unwrap();
        let a = &e["asker"];
        self.db()
            .execute(
                "INSERT INTO alerts (id, created_at, kind, severity, title, message, pane_id, tab_label, asked_by) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    id,
                    e["created_at"].as_str(),
                    e["alert_kind"].as_str(),
                    e["severity"].as_str(),
                    e["title"].as_str(),
                    e["message"].as_str(),
                    a["pane"].as_str(),
                    a["tab_label"].as_str(),
                    e["asked_by"].as_str()
                ],
            )
            .unwrap();
        if let Some(t) = a["terminal"].as_str() {
            self.db()
                .execute(
                    "INSERT INTO alert_host (id, host, terminal) VALUES (?, ?, ?)",
                    params![id, CORE, t],
                )
                .unwrap();
        }
    }

    fn machine(&self, m: &Value) {
        let error = m["offline"].as_str().map(|_| "unreachable");
        self.db()
            .execute(
                "INSERT OR IGNORE INTO remote_machines (machine_id, label, last_error) VALUES (?, ?, ?)",
                params![m["id"].as_str(), m["label"].as_str(), error],
            )
            .unwrap();
    }

    fn remote_group(&self, e: &Value) {
        self.machine(&e["machine"]);
        let key = e["key"].as_str().unwrap().strip_prefix("remote:").unwrap();
        let parts: Vec<&str> = key.split('|').collect();
        let group: i64 = parts[2].parse().unwrap();
        for q in e["questions"].as_array().unwrap() {
            let data = json!({
                "id": q["id"], "group": group, "qkey": format!("k{}", q["id"]), "status": "open",
                "question": q["question"], "options": q["options"], "header": q["header"],
                "asked_by": q["asked_by"], "context": q["context"], "title": e["title"],
                "tab_label": e["tab_label"], "created_at": e["created_at"],
            });
            self.remote_question(
                e["machine"]["id"].as_str().unwrap(),
                parts[1],
                q["id"].as_i64().unwrap(),
                "open",
                &data,
            );
        }
    }

    /// A raw mirror row.
    pub fn remote_question(&self, machine: &str, db: &str, id: i64, status: &str, data: &Value) {
        self.db()
            .execute(
                "INSERT INTO remote_questions (machine_id, db_id, remote_id, qkey, status, data_json, first_seen_at) \
                 VALUES (?, ?, ?, ?, ?, ?, '2026-01-02T09:00:00+00:00')",
                params![machine, db, id, format!("k{id}"), status, data.to_string()],
            )
            .unwrap();
    }

    fn remote_alert(&self, e: &Value) {
        self.machine(&e["machine"]);
        let data = json!({
            "id": e["id"], "akey": format!("a{}", e["id"]), "kind": e["alert_kind"], "severity": e["severity"],
            "status": "open", "title": e["title"], "message": e["message"], "tab_label": e["tab_label"],
            "asked_by": e["asked_by"], "created_at": e["created_at"],
        });
        self.db()
            .execute(
                "INSERT INTO remote_alerts (machine_id, db_id, alert_id, akey, status, data_json, first_seen_at) \
                 VALUES (?, 'db-alpha', ?, ?, 'open', ?, '2026-01-02T09:00:00+00:00')",
                params![
                    e["machine"]["id"].as_str(),
                    e["id"].as_i64(),
                    format!("a{}", e["id"]),
                    data.to_string()
                ],
            )
            .unwrap();
    }

    fn outbox(&self, e: &Value) {
        self.machine(&e["machine"]);
        let id = e["key"].as_str().unwrap().strip_prefix("routbox:").unwrap();
        self.db()
            .execute(
                "INSERT INTO remote_outbox (request_id, machine_id, db_id, remote_id, qkey, action, created_at, \
                 expires_at, state, result) VALUES (?, ?, 'db-alpha', ?, 'k', ?, ?, ?, ?, ?)",
                params![
                    id,
                    e["machine"]["id"].as_str(),
                    e["remote_id"].as_i64(),
                    e["action"].as_str(),
                    format!("2026-01-02T09:00:00+00:00{id}"),
                    e["expires_at"].as_str(),
                    e["state"].as_str(),
                    e["result"].as_str()
                ],
            )
            .unwrap();
    }

    fn problem(&self, e: &Value) {
        self.machine(&e["machine"]);
        let data = json!({
            "id": e["remote_id"], "status": "answered", "question": e["question"], "answer": e["answer"],
            "agent_gone": true,
        });
        self.remote_question(
            e["machine"]["id"].as_str().unwrap(),
            "db-alpha",
            e["remote_id"].as_i64().unwrap(),
            "answered",
            &data,
        );
    }

    /// Closed rows up to the feed's marks.
    fn marks(&self, feed: &Value) {
        let highest = |table: &str| -> i64 {
            self.db()
                .query_row(
                    &format!("SELECT COALESCE(MAX(id), 0) FROM {table}"),
                    [],
                    |r| r.get(0),
                )
                .unwrap()
        };
        let question = feed["new_since"]["question_id"].as_i64().unwrap();
        if question > highest("questions") {
            self.closed(question, question);
        }
        let alert = feed["new_since"]["alert_id"].as_i64().unwrap();
        if alert > highest("alerts") {
            self.db()
                .execute(
                    "INSERT INTO alerts (id, created_at, kind, severity, title, status) \
                     VALUES (?, '2026-01-02T09:00:00+00:00', 'agent', 'info', 'Question Title 123', 'acked')",
                    [alert],
                )
                .unwrap();
        }
    }
}
