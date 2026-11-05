//! [`Partial`]: what a pass over the store did, kept when a database error
//! stops it. Python notifies as it goes, so a later row's error never loses
//! the notification of an earlier one. Here the store hands its effects back
//! instead, and what a pass already committed must still reach the caller:
//! the alert, outbox row or `issue_notified_at` it changed is never picked up
//! again, so an effect dropped with the error would be lost for good.

/// What a pass collected before it ended, and the error that ended it early.
/// Carry out `value`, then look at `error`.
#[derive(Debug)]
#[must_use = "`value` holds effects of committed changes: dropping it loses them"]
pub struct Partial<T> {
    /// What was collected, complete or up to the error.
    pub value: T,
    /// The database error that stopped the pass, if one did.
    pub error: Option<rusqlite::Error>,
}

impl<T> Partial<T> {
    /// `value`, with how the pass that filled it `ended`.
    pub fn of(value: T, ended: rusqlite::Result<()>) -> Self {
        Partial {
            value,
            error: ended.err(),
        }
    }

    /// The value of a pass that ran to its end, else its error. Drops what
    /// was collected on an error: for a caller with nothing to carry out.
    pub fn into_result(self) -> rusqlite::Result<T> {
        match self.error {
            Some(e) => Err(e),
            None => Ok(self.value),
        }
    }
}
