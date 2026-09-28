//! `workflows/scripts/*-wf_*.js`: best-effort parsing of the `meta.phases` JS literal.

use nodal_domain::model::claude::PhaseInfo;

/// Phases declared in `meta.phases` of the workflow's JS script. Best effort: it's JS, not
/// JSON; looks for `title:` / `detail:` with string literals inside the array.
pub fn parse_script_phases(src: &str) -> Vec<PhaseInfo> {
    let Some(start) = src.find("phases:") else {
        return Vec::new();
    };
    let after = &src[start + "phases:".len()..];
    let Some(open) = after.find('[') else {
        return Vec::new();
    };
    if !after[..open].trim().is_empty() {
        return Vec::new();
    }
    let body = &after[open + 1..];
    let Some(end) = find_closing(body, '[', ']') else {
        return Vec::new();
    };
    split_top_level_objects(&body[..end])
        .into_iter()
        .filter_map(|obj| {
            Some(PhaseInfo {
                title: js_string_prop(obj, "title")?,
                detail: js_string_prop(obj, "detail"),
            })
        })
        .collect()
}

/// Index of the balancing closer, skipping string literals.
pub(crate) fn find_closing(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' | '`' => quote = Some(c),
            c if c == open => depth += 1,
            c if c == close => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Top-level `{ ... }` chunks, without the braces.
fn split_top_level_objects(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        let inner = &rest[open + 1..];
        let Some(end) = find_closing(inner, '{', '}') else {
            break;
        };
        out.push(&inner[..end]);
        rest = &inner[end + 1..];
    }
    out
}

/// Value of `key: '...'` (single quotes, double quotes or backticks).
pub(crate) fn js_string_prop(obj: &str, key: &str) -> Option<String> {
    let mut search = obj;
    loop {
        let pos = search.find(key)?;
        let before_ok = search[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
        let rest = search[pos + key.len()..].trim_start();
        if before_ok {
            if let Some(rest) = rest.strip_prefix(':') {
                let rest = rest.trim_start();
                let Some(q) = rest
                    .chars()
                    .next()
                    .filter(|c| matches!(c, '\'' | '"' | '`'))
                else {
                    search = &search[pos + key.len()..];
                    continue;
                };
                let mut out = String::new();
                let mut escaped = false;
                for c in rest[1..].chars() {
                    if escaped {
                        out.push(match c {
                            'n' => '\n',
                            't' => '\t',
                            other => other,
                        });
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == q {
                        return Some(out);
                    } else {
                        out.push(c);
                    }
                }
                return None;
            }
        }
        search = &search[pos + key.len()..];
    }
}

#[cfg(test)]
mod tests;
