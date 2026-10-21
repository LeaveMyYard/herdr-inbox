//! Python `herdr.panel_focused`.

use super::strip;

/// Python `str.splitlines()` (no line ends kept).
pub(super) fn splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        match rest.find(|c| {
            matches!(
                c,
                '\n' | '\r' | '\x0b' | '\x0c' | '\x1c'
                    ..='\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
            )
        }) {
            Some(k) => {
                out.push(&rest[..k]);
                let sep = rest[k..].chars().next().unwrap_or('\n');
                let skip = if rest[k..].starts_with("\r\n") {
                    2
                } else {
                    sep.len_utf8()
                };
                rest = &rest[k + skip..];
            }
            None => {
                out.push(rest);
                rest = "";
            }
        }
    }
    out
}

/// Claude Code's background-agents panel (or a subagent view) has the
/// keyboard: the first non-blank line below the screen's last `───` border
/// (an input box's) starts with `↑/↓ to select`. Only that footer counts:
/// the phrase in ordinary output never does. No border: no.
pub fn panel_focused(screen: &str) -> bool {
    let lines = splitlines(screen);
    let border = |line: &&str| {
        let s = strip(line);
        !s.is_empty() && s.chars().all(|c| c == '─')
    };
    let Some(last) = lines.iter().rposition(|l| border(l)) else {
        return false;
    };
    let footer = lines[last + 1..]
        .iter()
        .map(|l| strip(l))
        .find(|l| !l.is_empty())
        .unwrap_or("");
    footer.starts_with("↑/↓ to select")
}
