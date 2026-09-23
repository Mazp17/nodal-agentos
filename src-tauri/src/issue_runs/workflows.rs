//! Catálogo de workflows: `~/.claude/workflows/*.js` y `<repo>/.claude/workflows/*.js`.
//!
//! El `export const meta = {...}` es JS, no JSON, y no se ejecuta: se escanea el literal
//! respetando strings y comentarios y se leen `name`, `description` y `whenToUse` del
//! primer nivel. Si algo no se entiende, el workflow se lista igual con el nombre del archivo.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::runs::claude_fs::js_string_prop;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowSource {
    User,
    Repo,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInfo {
    pub name: String,
    pub description: Option<String>,
    pub when_to_use: Option<String>,
    pub source: WorkflowSource,
    pub path: String,
}

#[derive(Debug, Default, PartialEq)]
pub struct Meta {
    pub name: Option<String>,
    pub description: Option<String>,
    pub when_to_use: Option<String>,
}

/// Recorre `s` y devuelve una copia del mismo largo en bytes donde los comentarios y
/// todo lo anidado a profundidad > 0 quedan en blanco. Si `stop_at_close`, corta en la
/// `}` que cierra el nivel 0 y devuelve su índice.
fn scan_top_level(s: &str, stop_at_close: bool) -> (String, Option<usize>) {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    let mut i = 0;
    let blank = |out: &mut String, c: char| {
        // Mismo largo en bytes para que los índices de `out` sirvan sobre `s`.
        for _ in 0..c.len_utf8() {
            out.push(' ');
        }
    };
    while i < bytes.len() {
        let c = s[i..].chars().next().unwrap_or(' ');
        let len = c.len_utf8();
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c as u32 == q as u32 {
                quote = None;
            }
            if depth == 0 { out.push(c) } else { blank(&mut out, c) }
            i += len;
            continue;
        }
        // Comentarios: se blanquean (pueden tener comillas sueltas).
        if s[i..].starts_with("//") {
            let end = s[i..].find('\n').map_or(s.len(), |n| i + n);
            s[i..end].chars().for_each(|c| blank(&mut out, c));
            i = end;
            continue;
        }
        if s[i..].starts_with("/*") {
            let end = s[i + 2..].find("*/").map_or(s.len(), |n| i + 2 + n + 2);
            s[i..end].chars().for_each(|c| blank(&mut out, c));
            i = end;
            continue;
        }
        match c {
            '\'' | '"' | '`' => {
                quote = Some(c as u8);
                if depth == 0 { out.push(c) } else { blank(&mut out, c) }
            }
            '{' | '[' | '(' => {
                depth += 1;
                blank(&mut out, c);
            }
            '}' | ']' | ')' => {
                if depth == 0 {
                    if stop_at_close && c == '}' {
                        return (out, Some(i));
                    }
                } else {
                    depth -= 1;
                }
                blank(&mut out, c);
            }
            _ if depth == 0 => out.push(c),
            _ => blank(&mut out, c),
        }
        i += len;
    }
    (out, None)
}

/// Valor string de `key` en el primer nivel, con la clave sin comillas o entre comillas.
fn top_level_prop(flat: &str, key: &str) -> Option<String> {
    [key.to_string(), format!("\"{key}\""), format!("'{key}'")]
        .iter()
        .find_map(|k| js_string_prop(flat, k))
        .map(|v| v.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|v| !v.is_empty())
}

/// Lee `export const meta = { ... }`. `None` si no hay meta reconocible.
pub fn parse_meta(src: &str) -> Option<Meta> {
    let start = src.find("export const meta")?;
    let after = &src[start + "export const meta".len()..];
    let eq = after.find('=')?;
    if !after[..eq].trim().is_empty() && !after[..eq].trim_start().starts_with(':') {
        return None;
    }
    let rest = &after[eq + 1..];
    let open = rest.find('{')?;
    if !rest[..open].trim().is_empty() {
        return None;
    }
    let body = &rest[open + 1..];
    let (flat, close) = scan_top_level(body, true);
    close?;
    let meta = Meta {
        name: top_level_prop(&flat, "name"),
        description: top_level_prop(&flat, "description"),
        when_to_use: top_level_prop(&flat, "whenToUse"),
    };
    Some(meta)
}

/// Archivos de backup (`x.js.bak`, `x.js.bak-2026…`, `x.bak.js`) y ocultos no cuentan.
fn is_workflow_file(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else { return false };
    p.is_file() && name.ends_with(".js") && !name.starts_with('.') && !name.contains(".bak")
}

fn read_dir_workflows(dir: &Path, source: WorkflowSource) -> Vec<WorkflowInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| is_workflow_file(p)).collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let meta = std::fs::read_to_string(&path).ok().and_then(|s| parse_meta(&s)).unwrap_or_default();
            WorkflowInfo {
                name: meta.name.unwrap_or(stem),
                description: meta.description,
                when_to_use: meta.when_to_use,
                source,
                path: path.to_string_lossy().into_owned(),
            }
        })
        .collect()
}

/// Catálogo ordenado por nombre. Un workflow del repo pisa al del usuario con el mismo nombre.
pub fn list_from(user_dir: Option<&Path>, repo: Option<&Path>) -> Vec<WorkflowInfo> {
    let mut out: Vec<WorkflowInfo> = Vec::new();
    let user = user_dir.map(|d| read_dir_workflows(d, WorkflowSource::User)).unwrap_or_default();
    let repo = repo
        .map(|r| read_dir_workflows(&r.join(".claude").join("workflows"), WorkflowSource::Repo))
        .unwrap_or_default();
    for wf in user.into_iter().chain(repo) {
        match out.iter_mut().find(|w| w.name == wf.name) {
            Some(existing) => *existing = wf,
            None => out.push(wf),
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Nombre usable en `/<workflow> ...`: sin espacios ni nada que `claude` lea como opción.
pub fn is_valid_workflow_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && !name.starts_with('-')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/issue_runs/fixtures/workflows")
    }

    #[test]
    fn parses_real_linear_issue_meta() {
        let src = std::fs::read_to_string(fixtures().join("user/linear-issue.js")).unwrap();
        let m = parse_meta(&src).unwrap();
        assert_eq!(m.name.as_deref(), Some("linear-issue"));
        assert!(m.description.unwrap().starts_with("Lleva una issue de Linear"));
        assert!(m.when_to_use.unwrap().contains("Nunca mergea."));
    }

    #[test]
    fn parses_real_demo_board_meta() {
        let src = std::fs::read_to_string(fixtures().join("user/demo-board.js")).unwrap();
        let m = parse_meta(&src).unwrap();
        assert_eq!(m.name.as_deref(), Some("demo-board"));
        assert!(m.description.unwrap().starts_with("Workflow de juguete"));
        // Viene en la línea siguiente a `whenToUse:` y tiene comillas dobles adentro.
        assert!(m.when_to_use.unwrap().contains(r#"args: "volcanes""#));
    }

    #[test]
    fn nested_name_and_comments_do_not_confuse() {
        let src = "// meta = { name: 'falso' }\nexport const meta = {\n  /* name: 'otro' */\n  phases: [{ name: 'fase' }],\n  \"name\": \"real\",\n  description: 'con \\'escape\\' y ñandú',\n}\n";
        let m = parse_meta(src).unwrap();
        assert_eq!(m.name.as_deref(), Some("real"));
        assert_eq!(m.description.as_deref(), Some("con 'escape' y ñandú"));
        assert_eq!(m.when_to_use, None);
    }

    #[test]
    fn broken_or_missing_meta() {
        assert_eq!(parse_meta("const x = 1"), None);
        assert_eq!(parse_meta("export const meta = { name: 'sin cierre'"), None);
        assert_eq!(parse_meta("export const meta = loadMeta()"), None);
        let m = parse_meta("export const meta = { name: nombreVariable }").unwrap();
        assert_eq!(m.name, None);
    }

    #[test]
    fn catalog_ignores_backups_and_repo_overrides_user() {
        let root = fixtures();
        let list = list_from(Some(&root.join("user")), Some(&root.join("repo")));
        let names: Vec<_> = list.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["demo-board", "linear-issue", "sin-meta"]);
        let li = &list[1];
        assert_eq!(li.source, WorkflowSource::Repo);
        assert_eq!(li.description.as_deref(), Some("Variante del repo con \"comillas\" escapadas"));
        assert_eq!(list[2].description, None);

        let only_user = list_from(Some(&root.join("user")), None);
        assert_eq!(only_user.len(), 2);
        assert!(only_user.iter().all(|w| w.source == WorkflowSource::User));
        assert!(list_from(Some(Path::new("/no/existe")), None).is_empty());
    }

    /// Contra `~/.claude/workflows` de esta máquina: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_user_catalog() {
        let dir = crate::runs::claude_fs::claude_config_dir().unwrap().join("workflows");
        let list = list_from(Some(&dir), None);
        for w in &list {
            eprintln!("{} · {:?}", w.name, w.description);
        }
        assert!(list.iter().any(|w| w.name == "linear-issue" && w.description.is_some()));
        assert!(list.iter().all(|w| !w.path.contains(".bak")));
    }

    #[test]
    fn workflow_names() {
        assert!(is_valid_workflow_name("linear-issue"));
        assert!(is_valid_workflow_name("plugin:flow_2"));
        assert!(!is_valid_workflow_name("-x"));
        assert!(!is_valid_workflow_name("a b"));
        assert!(!is_valid_workflow_name(""));
    }
}
