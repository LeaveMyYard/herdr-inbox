//! Filing a call's questions (Python `service.ask_group`): each is deduped on
//! its own, inside one write transaction, so parallel calls (subagents) never
//! file the same question twice. A repeat of an open question gets its
//! receipt back; a repeat of a question answered within the reuse window gets
//! the answer (and the answer counts as seen). New ones share a group id and
//! the call's title. Afterwards, what Python does to herdr comes back as
//! [`Effect`]s. A core caller's rows have no pane and a `question_host` row
//! (`store::hosts`), and nothing is done to herdr for them.

use serde_json::{Value, json};

use super::notify::notification_body;
use crate::claude::Transcripts;
use crate::mcp::args::Item;
use crate::mcp::caller::Caller;
use crate::mcp::receipts;
use crate::store::Store;
use crate::store::alerts::Effect;
use crate::store::hosts::CORE_SERVER;
use crate::store::norm::normalize;
use crate::store::question::normalize_options;
use crate::store::questions::NewQuestion;
use crate::store::sessions::record_session;
use crate::store::tool_use;
use crate::text::py_isspace;

/// AskUserQuestion's chip limit, in characters (Python `service.MAX_HEADER`).
pub const MAX_HEADER: usize = 12;

/// The call as a whole: its questions and what they share.
pub struct Call<'a> {
    pub items: &'a [Item],
    pub context: Option<&'a str>,
    pub asked_by: Option<&'a str>,
    pub title: Option<&'a str>,
    /// The call's `tool_use_id`, recorded on every question it files.
    pub tool_use_id: Option<&'a str>,
}

fn open_receipt(qid: i64) -> Value {
    let r = format!("Q{qid}");
    json!({"id": qid, "status": "open", "message": format!(
        "Recorded as {r}. The operator will answer asynchronously; the answer will arrive in \
         this conversation as a message starting with `[inbox]` that names {r}. Continue with \
         work that does not depend on it. Do not repeat the question in chat or call ask_operator \
         for it again. If an answer seems overdue, call check_answers.")})
}

/// A header as stored: stripped, at most [`MAX_HEADER`] characters, an empty one none.
fn header(h: Option<&str>) -> Option<String> {
    let h: String = h
        .unwrap_or("")
        .trim_matches(py_isspace)
        .chars()
        .take(MAX_HEADER)
        .collect();
    (!h.is_empty()).then_some(h)
}

/// Files `call` for `caller` (asked on herdr server `server`): the receipt
/// the agent reads, and the effects to carry out after the commit.
pub fn ask_group(
    store: &Store,
    caller: &Caller,
    call: &Call<'_>,
    server: &str,
    files: &Transcripts,
) -> rusqlite::Result<(Value, Vec<Effect>)> {
    let (pane, seat) = (caller.pane_id.as_deref(), caller.core.as_ref());
    if pane.is_none() && seat.is_none() {
        // nobody watches the inbox for agents outside herdr: no popup, no notification, no delivery
        return Ok((receipts::unavailable(), Vec::new()));
    }
    let server = if seat.is_some() { CORE_SERVER } else { server };
    if let (Some(_), Some(session)) = (seat, caller.session_id.as_deref()) {
        // says which config directory this session's records are in: the core
        // worker's may be another, and it reads no process's environment
        let found = files.find(session, None).map(|p| p.display().to_string());
        record_session(store.db(), session, found.as_deref())?;
    }
    let owner = caller.owner();
    let (receipts, new, group_id, body) = store.immediate(|_| {
        let (mut receipts, mut new, mut group_id) = (Vec::new(), Vec::new(), None::<i64>);
        for item in call.items {
            let text = item.question.trim_matches(py_isspace);
            if let Some(dup) = store.find_duplicate(&normalize(text), owner, files)? {
                if dup.status == "open" {
                    store.bump_repeat(dup.id)?;
                    receipts.push(json!({"id": dup.id, "status": "already_open", "message": format!(
                        "Q{} is already with the operator. Do not ask again; keep working.", dup.id)}));
                } else {
                    store.acknowledge(&[dup.id])?;
                    let answer = dup.answer.clone().unwrap_or_default();
                    receipts.push(json!({"id": dup.id, "status": "answered", "answer": dup.answer,
                        "message": format!("Q{} was already answered by the operator: {answer}", dup.id)}));
                }
                continue;
            }
            let options = item.options.as_ref().map(|o| Value::Array(o.clone()));
            let options = match options.as_ref().map(normalize_options) {
                Some(Err(e)) => return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(e))),
                Some(Ok(o)) => o,
                None => None,
            };
            let header = header(item.header.as_deref());
            let q = store.insert(&NewQuestion {
                question: text,
                options,
                context: call.context,
                asked_by: call.asked_by,
                pane_id: pane,
                tab_id: caller.tab_id.as_deref(),
                tab_label: caller.tab_label.as_deref(),
                session_id: caller.session_id.as_deref(),
                cwd: caller.cwd.as_deref(),
                header: header.as_deref(),
                multi_select: item.multi_select,
                group_id,
            })?;
            store.set_server(q.id, Some(server))?; // this server's delivery worker owns it
            if let Some(seat) = seat {
                store.set_question_host(q.id, seat)?;
            }
            if let Some(id) = tool_use::checked(call.tool_use_id) {
                store.set_tool_use_id(q.id, id)?;
            }
            group_id = Some(q.group());
            receipts.push(open_receipt(q.id));
            new.push(q);
        }
        let mut body = None;
        if let (false, Some(g)) = (new.is_empty(), group_id) {
            store.set_group_title(g, call.title)?;
            // read here, not after the commit: a failed read must not re-run a call that filed
            body = Some(notification_body(&store.group(g, false)?));
        }
        Ok((receipts, new, group_id, body))
    })?;
    let mut effects = Vec::new();
    if let (Some(first), Some(body), Some(pane)) = (new.first(), body, pane) {
        let refs: Vec<String> = new.iter().map(|q| format!("Q{}", q.id)).collect();
        // Python `tab_label or pane_id`: an empty label is none
        let place = [&first.tab_label, &first.pane_id]
            .into_iter()
            .flatten()
            .find(|s| !s.is_empty())
            .map_or("", String::as_str);
        effects.push(Effect::Notify {
            title: format!("{} · {place}", refs.join(", ")),
            body,
        });
        effects.push(Effect::Markers(Some(pane.to_owned())));
    }
    let ids: Vec<String> = receipts
        .iter()
        .map(|r| format!("Q{} ({})", r["id"], r["status"].as_str().unwrap_or("")))
        .collect();
    Ok((
        json!({
            "group": group_id,
            "questions": receipts,
            "message": format!(
                "Filed {}. Answers arrive later as `[inbox]` messages naming the Q ids; keep working \
                 and do not ask these again. If an answer seems overdue, call check_answers.",
                ids.join(", ")
            ),
        }),
        effects,
    ))
}

/// One question (`question`, not `questions`): the group's receipt for it
/// alone, in its single-question shape (Python `service.ask`).
pub fn single(group_receipt: Value) -> Value {
    match group_receipt.get("questions").and_then(|q| q.get(0)) {
        Some(first) => first.clone(),
        None => group_receipt,
    }
}
