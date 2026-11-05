//! Python's `shlex.shlex(command, posix=True, punctuation_chars=";&|\n")`
//! with `whitespace = " \t\r"` and `whitespace_split = True`, as
//! `transcript.own_answer_qids` splits a shell command: quotes and escapes
//! as a POSIX shell reads them, `#` comments (to the end of the line, the
//! newline included), and runs of `;`, `&`, `|` and newlines as tokens of
//! their own. State for state CPython's `read_token`.

const PUNCTUATION: &str = ";&|\n";
const WHITESPACE: &str = " \t\r";
const QUOTES: &str = "'\"";

/// A command Python's shlex refuses (an unclosed quote, a trailing escape).
#[derive(Debug, PartialEq, Eq)]
pub struct Unbalanced;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Space,
    Word,
    Punct,
    Quote(char),
    Escape,
    End,
}

/// The tokens, in order (an empty quoted word is an empty token).
pub fn split(command: &str) -> Result<Vec<String>, Unbalanced> {
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0;
    let mut pushback: Option<char> = None;
    let mut out = Vec::new();
    let mut state = State::Space;
    loop {
        if state == State::End {
            return Ok(out);
        }
        let mut token = String::new();
        let mut quoted = false;
        let mut escaped = State::Space;
        loop {
            let next = pushback.take().or_else(|| {
                let c = chars.get(i).copied();
                i += usize::from(c.is_some());
                c
            });
            let comment = |i: &mut usize| {
                // `instream.readline()`: the rest of the line, its newline too
                while let Some(&c) = chars.get(*i) {
                    *i += 1;
                    if c == '\n' {
                        break;
                    }
                }
            };
            match state {
                State::End => break,
                State::Space => match next {
                    None => {
                        state = State::End;
                        break;
                    }
                    Some(c) if WHITESPACE.contains(c) => {
                        if !token.is_empty() || quoted {
                            break;
                        }
                    }
                    Some('#') => comment(&mut i),
                    Some('\\') => {
                        escaped = State::Word;
                        state = State::Escape;
                    }
                    Some(c) if PUNCTUATION.contains(c) => {
                        token.push(c);
                        state = State::Punct;
                    }
                    Some(c) if QUOTES.contains(c) => state = State::Quote(c),
                    Some(c) => {
                        token.push(c);
                        state = State::Word;
                    }
                },
                State::Word | State::Punct => match next {
                    None => {
                        state = State::End;
                        break;
                    }
                    Some(c) if WHITESPACE.contains(c) => {
                        state = State::Space;
                        if !token.is_empty() || quoted {
                            break;
                        }
                    }
                    Some('#') => {
                        comment(&mut i);
                        state = State::Space;
                        if !token.is_empty() || quoted {
                            break;
                        }
                    }
                    Some(c) if state == State::Punct => {
                        if PUNCTUATION.contains(c) {
                            token.push(c);
                        } else {
                            pushback = Some(c);
                            state = State::Space;
                            break;
                        }
                    }
                    Some(c) if QUOTES.contains(c) => state = State::Quote(c),
                    Some('\\') => {
                        escaped = State::Word;
                        state = State::Escape;
                    }
                    Some(c) if !PUNCTUATION.contains(c) => token.push(c),
                    Some(c) => {
                        pushback = Some(c);
                        state = State::Space;
                        if !token.is_empty() || quoted {
                            break;
                        }
                    }
                },
                State::Quote(q) => {
                    quoted = true;
                    match next {
                        None => return Err(Unbalanced),
                        Some(c) if c == q => state = State::Word,
                        // only `"` lets an escape work inside (shlex `escapedquotes`)
                        Some('\\') if q == '"' => {
                            escaped = State::Quote(q);
                            state = State::Escape;
                        }
                        Some(c) => token.push(c),
                    }
                }
                State::Escape => {
                    let Some(c) = next else {
                        return Err(Unbalanced);
                    };
                    // inside quotes only the quote or the escape itself is escaped
                    if let State::Quote(q) = escaped
                        && c != q
                        && c != '\\'
                    {
                        token.push('\\');
                    }
                    token.push(c);
                    state = escaped;
                }
            }
        }
        // posix: an unquoted empty token is the end (`None`), never a token
        if !token.is_empty() || quoted {
            out.push(token);
        } else if state == State::End {
            return Ok(out);
        }
    }
}

#[cfg(test)]
mod tests;
