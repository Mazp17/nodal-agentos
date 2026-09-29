//! `claude --bg` output: the `backgrounded · <id>` line.

/// Extracts the short id from a line like `backgrounded · ddb91222` (with or without ANSI colors).
pub fn parse_bg_line(line: &str) -> Option<String> {
    let clean = strip_ansi(line);
    let pos = clean.find("backgrounded")?;
    let rest = &clean[pos + "backgrounded".len()..];
    // First token after `backgrounded` (there may be text after the id).
    let id = rest
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .find(|t| !t.is_empty())?;
    is_plausible_id(id).then(|| id.to_string())
}

/// Fallback if the output lacks the word `backgrounded` (e.g. another format without a TTY):
/// accept output that is just a hex id. Not observed, purely defensive.
pub fn parse_bare_id(output: &str) -> Option<String> {
    let clean = strip_ansi(output);
    let t = clean.trim();
    (t.len() >= 6 && t.len() <= 16 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| t.to_string())
}

fn is_plausible_id(id: &str) -> bool {
    (4..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // CSI: parameters up to a final letter.
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() || n == '~' {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests;
