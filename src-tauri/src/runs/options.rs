//! Validación de los flags de lanzamiento por repo (`--model`, `--effort`,
//! `--permission-mode`) y su traducción a argumentos de `claude`.
//! Valores verificados contra `claude --help` (v2.1.281).

use super::types::LaunchOptions;

pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
/// `claude --help` lista todos menos `default`, que también se acepta (verificado: alias de `manual`).
pub const PERMISSION_MODES: [&str; 7] =
    ["default", "manual", "acceptEdits", "auto", "dontAsk", "plan", "bypassPermissions"];
/// Alias de "último modelo" que acepta `--model`.
pub const MODEL_ALIASES: [&str; 4] = ["fable", "opus", "sonnet", "haiku"];

/// Alias conocido, o nombre completo `claude-...` (con sufijo `[1m]` opcional).
pub fn is_valid_model(m: &str) -> bool {
    if MODEL_ALIASES.contains(&m) {
        return true;
    }
    let base = m.strip_suffix("[1m]").unwrap_or(m);
    base.len() <= 64
        && base.len() > "claude-".len()
        && base.starts_with("claude-")
        && base.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
}

fn clean(v: &Option<String>) -> Option<String> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// Normaliza (trim, vacío → `None`) y valida contra las listas permitidas.
pub fn normalize(opts: &LaunchOptions) -> Result<LaunchOptions, Vec<String>> {
    let out = LaunchOptions {
        model: clean(&opts.model),
        effort: clean(&opts.effort),
        permission_mode: clean(&opts.permission_mode),
    };
    let mut errors = Vec::new();
    if let Some(m) = &out.model {
        if !is_valid_model(m) {
            errors.push(format!(
                "Invalid model \"{m}\": use {} or a full name like claude-sonnet-5.",
                MODEL_ALIASES.join(", ")
            ));
        }
    }
    if let Some(e) = &out.effort {
        if !EFFORTS.contains(&e.as_str()) {
            errors.push(format!("Invalid effort \"{e}\": use {}.", EFFORTS.join(", ")));
        }
    }
    if let Some(p) = &out.permission_mode {
        if !PERMISSION_MODES.contains(&p.as_str()) {
            errors.push(format!("Invalid permission mode \"{p}\": use {}.", PERMISSION_MODES.join(", ")));
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// Argumentos (cada uno por separado, sin shell) para `claude --bg`.
pub fn to_args(opts: &LaunchOptions) -> Result<Vec<String>, String> {
    let opts = normalize(opts).map_err(|e| e.join("\n"))?;
    let mut args = Vec::new();
    if let Some(m) = opts.model {
        args.push("--model".into());
        args.push(m);
    }
    if let Some(e) = opts.effort {
        args.push("--effort".into());
        args.push(e);
    }
    if let Some(p) = opts.permission_mode {
        args.push("--permission-mode".into());
        args.push(p);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(model: Option<&str>, effort: Option<&str>, perm: Option<&str>) -> LaunchOptions {
        LaunchOptions {
            model: model.map(String::from),
            effort: effort.map(String::from),
            permission_mode: perm.map(String::from),
        }
    }

    #[test]
    fn models() {
        for ok in ["opus", "sonnet", "fable", "haiku", "claude-sonnet-5", "claude-opus-4-1", "claude-sonnet-4.5", "claude-opus-5[1m]"] {
            assert!(is_valid_model(ok), "{ok}");
        }
        for bad in ["", "gpt-4", "claude-", "--model", "claude-x; rm -rf ~", "Claude-Sonnet", "claude-a b"] {
            assert!(!is_valid_model(bad), "{bad}");
        }
    }

    #[test]
    fn args_are_separate_and_only_when_set() {
        assert!(to_args(&LaunchOptions::default()).unwrap().is_empty());
        assert_eq!(
            to_args(&opts(Some(" sonnet "), Some("xhigh"), Some("acceptEdits"))).unwrap(),
            ["--model", "sonnet", "--effort", "xhigh", "--permission-mode", "acceptEdits"]
        );
        assert_eq!(to_args(&opts(Some(""), None, Some("plan"))).unwrap(), ["--permission-mode", "plan"]);
    }

    #[test]
    fn rejects_unknown_values() {
        let err = normalize(&opts(Some("gpt"), Some("ultra"), Some("yolo"))).unwrap_err();
        assert_eq!(err.len(), 3);
        assert!(to_args(&opts(None, Some("--help"), None)).is_err());
    }
}
