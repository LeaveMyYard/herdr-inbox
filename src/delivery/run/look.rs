//! The lookups of one pane's pass (`inbox_core::deliver::Look` for Python
//! `deliver_batch`'s calls): each made when the decision asks, against the
//! store, herdr and the transcripts. A database error is kept and handed
//! back after the decision; meanwhile the lookup answers as if nothing were
//! there (not claimed, no misses), so nothing more is typed or written.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use herdr_agent_kit::agent_session::SessionId;
use herdr_agent_kit::herdr_api::AgentInfo;
use herdr_agent_kit::plugin_env::LockFile;
use inbox_core::deliver::{Agent, Mode};
use serde_json::Value;

use super::relocate::{MOVED, session_pane, tab_name};
use super::typed::Typed;
use super::{BUSY_GRACE, Claims, Ctx, HOOK_LIVE};
use crate::claude::landed::{Expected, landed_kinds};
use crate::clock::now_minus;
use crate::delivery::pane::{live_agent, panel};
use crate::delivery::plans::{older_conversation, question_family};
use crate::delivery::{expected_segments, landed_across};
use crate::store::Question;
use crate::store::sessions::session_transcript;

/// One pane's pass: what it looked up and took so far.
pub struct PaneLook<'c, 'a> {
    pub(super) ctx: &'c mut Ctx<'a>,
    pub(super) pane: String,
    pub(super) qs: BTreeMap<i64, &'c Question>,
    pub(super) mode: Mode,
    pub(super) claims: Claims,
    pub(super) agent: Option<Agent>,
    agents: Option<Vec<AgentInfo>>,
    /// The followed session's transcript (found before typing, Python `path`).
    pub(super) path: Option<PathBuf>,
    pub(super) typed: Vec<Typed>,
    pub(super) error: Option<rusqlite::Error>,
}

impl<'c, 'a> PaneLook<'c, 'a> {
    pub fn new(ctx: &'c mut Ctx<'a>, pane: &str, items: &[&'c Question], mode: Mode) -> Self {
        PaneLook {
            ctx,
            pane: pane.to_string(),
            qs: items.iter().map(|q| (q.id, *q)).collect(),
            mode,
            claims: Claims::new(),
            agent: None,
            agents: None,
            path: None,
            typed: Vec::new(),
            error: None,
        }
    }

    /// The claims taken, the verify-later jobs, and the first database error.
    pub fn finish(mut self) -> (Claims, Vec<Value>, Option<rusqlite::Error>) {
        let jobs = if self.mode.verify_later {
            self.jobs()
        } else {
            Vec::new()
        };
        (self.claims, jobs, self.error)
    }

    /// `r`'s value, or `T::default()` with the error kept.
    pub(super) fn ok<T: Default>(&mut self, r: rusqlite::Result<T>) -> T {
        r.unwrap_or_else(|e| {
            self.error.get_or_insert(e);
            T::default()
        })
    }

    pub(super) fn q(&self, id: i64) -> &'c Question {
        self.qs[&id]
    }

    pub(super) fn session(&self) -> Option<&str> {
        self.agent.as_ref().and_then(|a| a.session.as_deref())
    }

    /// A session's transcript (Python `transcript.find(s, store.session_transcript(s))`).
    pub(super) fn find(&mut self, session: Option<&str>) -> Option<PathBuf> {
        let s = session.filter(|s| !s.is_empty())?;
        let known = session_transcript(self.ctx.store.db(), s);
        let known = self.ok(known.map(Some))?;
        self.ctx.files.find(s, known.as_deref())
    }

    /// Python `_watched`: the idle session's asyncRewake watcher stamped
    /// itself recently and holds its lock.
    fn watched_now(&mut self) -> bool {
        let session = self.session().map(String::from);
        let live = self.ctx.store.watch_live(session.as_deref(), HOOK_LIVE);
        if !self.ok(live) {
            return false;
        }
        let Some(sid) = session.as_deref().and_then(SessionId::parse) else {
            return false;
        };
        LockFile::new(
            self.ctx
                .state_dir
                .join("rewake")
                .join(format!("{sid}.lock")),
        )
        .held_elsewhere_if_exists()
    }
}

impl inbox_core::deliver::Look for PaneLook<'_, '_> {
    fn claimed(&mut self, id: i64) -> bool {
        let claim = self.ctx.store.claim_delivery(id);
        match self.ok(claim) {
            Some(c) => {
                self.claims.insert(id, c);
                true
            }
            None => false,
        }
    }
    fn agent(&mut self) -> Option<Agent> {
        self.agent = live_agent(
            self.ctx.herdr,
            self.ctx.files,
            self.ctx.state_dir,
            &self.pane,
        );
        self.agent.clone()
    }
    fn moved_elsewhere(&mut self, id: i64) -> bool {
        if self.agents.is_none() {
            self.agents = Some(self.ctx.herdr.agents().unwrap_or_default());
        }
        let q = self.q(id);
        let agents = self.agents.clone().unwrap_or_default();
        let loc = session_pane(self.ctx, q.session_id.as_deref(), &agents);
        let Some((pane, tab)) = self.ok(loc) else {
            return false;
        };
        if pane == self.pane {
            return false;
        }
        let label = tab_name(self.ctx, tab.as_deref());
        let moved = self.ctx.store.move_to_pane(
            id,
            &pane,
            tab.as_deref(),
            &format!("{MOVED} {pane}"),
            self.claims.get(&id),
            label.as_deref(),
        );
        self.ok(moved)
    }
    fn conversation_changed(&mut self, id: i64) -> bool {
        let q = self.q(id);
        if q.session_id.as_deref().is_none_or(str::is_empty) {
            return false;
        }
        let family = question_family(self.ctx.store.db(), self.ctx.files, q);
        let session = self.session().map(String::from);
        !self
            .ok(family)
            .iter()
            .any(|s| Some(s.as_str()) == session.as_deref())
    }
    fn pane_runs_older_session(&mut self, id: i64) -> bool {
        let (q, session) = (self.q(id), self.session().map(String::from));
        let older = older_conversation(
            self.ctx.store.db(),
            self.ctx.files,
            session.as_deref(),
            q.session_id.as_deref(),
        );
        self.ok(older)
    }
    fn watched(&mut self) -> bool {
        self.watched_now()
    }
    fn hook_misses(&mut self, id: i64) -> i64 {
        let n = self.ctx.store.hook_misses(id);
        self.ok(n)
    }
    fn hook_live(&mut self) -> bool {
        let session = self.session().map(String::from);
        let live = self.ctx.store.hook_live(session.as_deref(), HOOK_LIVE);
        self.ok(live)
    }
    fn busy_attempts(&mut self, id: i64) -> i64 {
        let n = self.ctx.store.busy_attempts(id);
        self.ok(n)
    }
    fn within_busy_grace(&mut self, id: i64) -> bool {
        // Python `_busy_for(...) < BUSY_GRACE`: never pending counts as 0 s
        let since = self.ctx.store.pending_since(id);
        self.ok(since)
            .filter(|s| !s.is_empty())
            .is_none_or(|s| s > now_minus(BUSY_GRACE))
    }
    fn panel_focused(&mut self) -> bool {
        panel(self.ctx.herdr, &self.pane)
    }
    fn landed(&mut self, ids: &[i64]) -> Vec<Option<Vec<usize>>> {
        let session = self.session().map(String::from);
        self.path = self.find(session.as_deref());
        let ours: Vec<Question> = ids.iter().map(|id| self.q(*id).clone()).collect();
        let found = landed_across(
            self.ctx.store.db(),
            self.ctx.files,
            &ours,
            std::slice::from_ref(&self.path),
        );
        let found = self.ok(found);
        ids.iter()
            .map(|id| found.get(id).map(|s| s.iter().copied().collect()))
            .collect()
    }
    fn pane_reports_another_session(&mut self) -> bool {
        let agent = self.agent.as_ref();
        let reported = agent
            .and_then(|a| a.reported_session.as_deref())
            .filter(|r| !r.is_empty());
        reported.is_some_and(|r| Some(r) != agent.and_then(|a| a.session.as_deref()))
    }
    fn already_typed_into_older_session(&mut self, id: i64) -> bool {
        let q = self.q(id);
        let reported = self.agent.as_ref().and_then(|a| a.reported_session.clone());
        if q.session_id.as_deref().is_none_or(str::is_empty) {
            return false;
        }
        let older = older_conversation(
            self.ctx.store.db(),
            self.ctx.files,
            reported.as_deref(),
            q.session_id.as_deref(),
        );
        if !self.ok(older) {
            return false;
        }
        let Some(older_path) = self.find(reported.as_deref()) else {
            return false;
        };
        let expected = Expected {
            segments: expected_segments(q),
            since: q.answered_at.clone(),
        };
        let id = u64::try_from(id).unwrap_or(0);
        landed_kinds(&older_path, &BTreeMap::from([(id, expected)])).contains_key(&id)
    }
    fn messages(&mut self, ids: &[i64]) -> Vec<(Vec<i64>, bool)> {
        self.batches(ids)
    }
    fn agent_again(&mut self, _: &[i64]) -> Option<Agent> {
        live_agent(
            self.ctx.herdr,
            self.ctx.files,
            self.ctx.state_dir,
            &self.pane,
        )
    }
    fn composer(&mut self, ids: &[i64]) -> inbox_core::deliver::Composer {
        self.composer_state(ids, Instant::now())
    }
    fn prompt(&mut self, ids: &[i64]) -> Option<String> {
        self.type_batch(ids)
    }
    fn verify_transcript(&mut self, ids: &[i64]) -> bool {
        self.typed_of(ids).is_some_and(|t| t.path.is_some())
    }
    fn landing(&mut self, id: i64) -> inbox_core::verify::Landing {
        self.landing_of(id)
    }
}
