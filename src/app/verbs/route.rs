//! After the commit (`app-commands-v1.md`, "Delivery"): the worker of the
//! answer's own server first, then whether a sender can go, then the detached
//! sender. **SAFETY**: the verb's process never types, a verb never says
//! `queued` when no sender can go, and skipping the sender never skips the worker.

use crate::app::feed::asker;
use crate::store::Store;
use crate::store::alerts::Alert;
use crate::store::hosts::{CORE, CORE_SERVER};
use crate::store::question::Question;

/// What starts after a commit. There is no way to send here: a verb's process
/// starts the sender and leaves.
pub trait Effects {
    /// Makes sure the worker of `server` runs (`None`: the verb's own).
    fn ensure_worker(&mut self, server: Option<&str>);
    /// herdr proves `server` is not running. Only `HerdrError::NotRunning`
    /// does: any other failure is no proof.
    fn server_gone(&mut self, server: Option<&str>) -> bool;
    /// A proven absence of herdr-core.
    fn core_gone(&mut self) -> bool;
    /// Starts `deliver <questions> [--force]` detached, with `socket` as its
    /// `HERDR_SOCKET_PATH` (`None`: the inherited one). One that fails to start
    /// leaves the answers to the worker just ensured.
    fn start_sender(&mut self, questions: &[i64], force: bool, socket: Option<&str>);
    /// Refreshes the panes' `$inbox_q` and `❓N`. Cosmetic: it never fails a verb.
    fn refresh_marks(&mut self, marks: &[Mark]);
}

/// Who can bring an answer to its agent, read in the verb's transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// A herdr pane.
    Tui {
        /// The stored server, if it was stored.
        server: Option<String>,
        /// The question has a session, so `deliver` can check the conversation.
        session: bool,
    },
    /// A core terminal.
    Core,
    /// Neither a pane nor a host record: only `check_answers`.
    Nowhere {
        /// The stored server, if any.
        server: Option<String>,
    },
}

/// Some questions and how to reach them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Send {
    /// How.
    pub route: Route,
    /// Which, in order.
    pub questions: Vec<i64>,
}

/// A pane whose marks moved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    /// Its server (`None`: the verb's own).
    pub server: Option<String>,
    /// The pane.
    pub pane: String,
}

/// The route of `q`: from the database, never from the mod.
pub fn route_of(store: &Store, q: &Question) -> rusqlite::Result<Route> {
    let server = store.server_of(q.id)?.filter(|s| !s.is_empty());
    let core = asker::of_question(store, q)?["host"] == CORE;
    Ok(if core || server.as_deref() == Some(CORE_SERVER) {
        Route::Core
    } else if q.pane_id.as_deref().is_some_and(|p| !p.is_empty()) {
        let session = q.session_id.as_deref().is_some_and(|s| !s.is_empty());
        Route::Tui { server, session }
    } else {
        Route::Nowhere { server }
    })
}

/// `questions` by route, in the order the routes first appear. One call has
/// one asker, so this is one entry unless a row was moved.
pub fn plan(store: &Store, questions: &[&Question]) -> rusqlite::Result<Vec<Send>> {
    let mut out: Vec<Send> = Vec::new();
    for q in questions {
        let route = route_of(store, q)?;
        match out.iter_mut().find(|s| s.route == route) {
            Some(send) => send.questions.push(q.id),
            None => out.push(Send {
                route,
                questions: vec![q.id],
            }),
        }
    }
    Ok(out)
}

impl Mark {
    /// The panes of `questions`, once each.
    pub fn of(store: &Store, questions: &[&Question]) -> rusqlite::Result<Vec<Mark>> {
        let mut out: Vec<Mark> = Vec::new();
        for q in questions {
            let Route::Tui { server, .. } = route_of(store, q)? else {
                continue;
            };
            let Some(pane) = q.pane_id.clone() else {
                continue;
            };
            let mark = Mark { server, pane };
            if !out.contains(&mark) {
                out.push(mark);
            }
        }
        Ok(out)
    }

    /// The pane of alert `a`.
    pub fn of_alert(_store: &Store, a: &Alert) -> rusqlite::Result<Vec<Mark>> {
        let server = a
            .herdr_server
            .clone()
            .filter(|s| !s.is_empty() && s != CORE_SERVER);
        let pane = a.pane_id.clone().filter(|p| !p.is_empty());
        Ok(pane.map(|pane| Mark { server, pane }).into_iter().collect())
    }
}

/// Starts what `send` needs, after the commit: every worker first, so a sender
/// lost after this is retried. `delivery`: `held` if any answer has no sender,
/// `None` when nothing waits. `retry` forces a pane's sender, never a core's.
pub fn deliver(
    send: &[Send],
    retry: bool,
    core_delivery: bool,
    fx: &mut dyn Effects,
) -> Option<&'static str> {
    if send.is_empty() {
        return None;
    }
    for s in send {
        match &s.route {
            Route::Tui { server, .. } | Route::Nowhere { server } => {
                fx.ensure_worker(server.as_deref());
            }
            Route::Core if core_delivery => fx.ensure_worker(Some(CORE_SERVER)),
            Route::Core => {}
        }
    }
    let mut held = false;
    for s in send {
        match &s.route {
            Route::Tui { server, session } => {
                let recipient_known = server.is_some() || *session;
                if recipient_known && !fx.server_gone(server.as_deref()) {
                    fx.start_sender(&s.questions, retry, server.as_deref());
                } else {
                    held = true;
                }
            }
            Route::Core if core_delivery && !fx.core_gone() => {
                fx.start_sender(&s.questions, false, None);
            }
            Route::Core | Route::Nowhere { .. } => held = true,
        }
    }
    Some(if held { "held" } else { "queued" })
}

#[cfg(test)]
mod tests;
