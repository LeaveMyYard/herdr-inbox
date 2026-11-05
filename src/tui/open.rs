//! What the popup opens on (Python `InboxApp.on_mount`): the operator's
//! pane, the database, and a deep link if `inbox open --question …` asked
//! for one.
//!
//! The order is Python's ([`crate::deeplink`]): the caller takes the popup
//! lock first; then the request is taken (once, by the popup holding its
//! nonce) and resolved against the database. A target that is gone opens
//! the popup as usual with a notice ("Q4 is no longer open"); the model
//! decides the rest (a target that is not a row of its list is ignored).

use herdr_agent_kit::plugin_env::lock::HeldLock;
use inbox_core::popup::{DeepLink, Entry};

use crate::deeplink::describe;
use crate::deeplink::lock::hold_popup_lock;
use crate::deeplink::request::{consume_request, unix_now};
use crate::deeplink::resolve::{Target, resolve};
use crate::store::Store;

/// The pane the popup returns to (Python: `herdr.invocation_pane()`, else
/// `$INBOX_FOCUS_PANE`, else the pane herdr reports focused). `invocation`
/// is the first, `env` the second, `focused` asks herdr (only if needed).
pub fn focus_pane(
    invocation: Option<String>,
    env: Option<String>,
    focused: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let set = |p: Option<String>| p.filter(|p| !p.is_empty());
    set(invocation)
        .or_else(|| set(env))
        .or_else(|| set(focused()))
}

/// Takes the popup lock for this popup's life (`inbox open` then reports an
/// open popup), then the deep link it was opened for. Drop the lock when
/// the popup closes.
pub fn take(
    store: &Store,
    state_dir: &std::path::Path,
    nonce: Option<&str>,
) -> (Option<HeldLock>, Option<DeepLink>) {
    let lock = hold_popup_lock(state_dir);
    (lock, deeplink(store, state_dir, nonce))
}

/// The deep link this popup was opened for, if any: the request named by
/// `nonce` in `state_dir`, resolved in `store`.
///
/// A remote question (`machine:Qn`) is not taken up yet (it needs herdr's
/// machine list here): such a request opens the popup as usual.
pub fn deeplink(
    store: &Store,
    state_dir: &std::path::Path,
    nonce: Option<&str>,
) -> Option<DeepLink> {
    let request = consume_request(state_dir, nonce, unix_now())?;
    if request.kind == "question" && request.value.contains(':') {
        return None;
    }
    // a database that cannot be read: the popup opens as usual (a deep link is
    // never worth a broken popup)
    let found = resolve(store, &request, Vec::new).ok()?;
    let (target, qid) = match found {
        Some((Target::Group(g), qid)) => (Some(Entry::Group(g)), qid),
        Some((Target::Remote(_), _)) => return None,
        None => (None, None),
    };
    Some(DeepLink {
        missing: if target.is_none() {
            format!(
                "{} is no longer open",
                describe(&request.kind, &request.value)
            )
        } else {
            String::new()
        },
        target,
        qid,
        hold: request.hold,
    })
}

#[cfg(test)]
mod tests;
