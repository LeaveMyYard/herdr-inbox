//! The size limits on long text (`app-feed-v1.md`, "Delivery"). A cut stops at
//! a character boundary, counted in UTF-8 bytes. Labels, drafts, `asked_by`,
//! `cwd` and `tab_label` are never cut. The limits are part of the tokens'
//! input: a token hashes text as published.

/// `question`, `context` and `answer`.
pub const LONG: usize = 16 * 1024;
/// `preview`.
pub const PREVIEW: usize = 4 * 1024;
/// `description`.
pub const DESCRIPTION: usize = 1024;

/// How a row's long text comes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Text {
    /// Cut to the limits, as the feed publishes it.
    Cut,
    /// Whole: `app show`.
    Whole,
}

impl Text {
    /// `s`, cut to `limit` bytes unless whole. Sets `cut` when it was.
    pub fn apply(self, s: &str, limit: usize, cut: &mut bool) -> String {
        if self == Text::Whole || s.len() <= limit {
            return s.to_string();
        }
        *cut = true;
        let mut end = limit;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s[..end].to_string()
    }

    /// [`Text::apply`] on a text that may be absent.
    pub fn apply_opt(self, s: Option<&str>, limit: usize, cut: &mut bool) -> Option<String> {
        s.map(|s| self.apply(s, limit, cut))
    }
}

#[cfg(test)]
mod tests;
