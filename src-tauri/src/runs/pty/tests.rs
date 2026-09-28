use super::*;

#[test]
fn ids_increase_monotonically_and_start_at_one() {
    let sessions = PtySessions::default();
    assert_eq!(sessions.next_id(), Ok(1));
    assert_eq!(sessions.next_id(), Ok(2));
    assert_eq!(sessions.next_id(), Ok(3));
}

#[test]
fn next_id_errors_instead_of_wrapping_to_zero() {
    let sessions = PtySessions::default();
    sessions.next_id.store(u32::MAX, Ordering::Relaxed);
    assert!(sessions.next_id().is_err());
    // Doesn't creep past `u32::MAX` on repeated calls either.
    assert!(sessions.next_id().is_err());
}

#[test]
fn locale_fallback_when_unset_blank_c_posix_or_non_utf8() {
    assert!(needs_default_locale(|_| None));
    assert!(needs_default_locale(|k| if k == "LANG" {
        Some(String::new())
    } else {
        None
    }));
    assert!(needs_default_locale(|k| if k == "LANG" {
        Some("C".into())
    } else {
        None
    }));
    assert!(needs_default_locale(|k| if k == "LC_ALL" {
        Some("POSIX".into())
    } else {
        None
    }));
    assert!(needs_default_locale(|k| if k == "LANG" {
        Some("en_US.ISO8859-1".into())
    } else {
        None
    }));
    assert!(!needs_default_locale(|k| if k == "LANG" {
        Some("en_US.UTF-8".into())
    } else {
        None
    }));
    assert!(!needs_default_locale(|k| if k == "LC_CTYPE" {
        Some("C.UTF-8".into())
    } else {
        None
    }));
}

#[test]
fn locale_precedence_is_lc_all_then_lc_ctype_then_lang() {
    // A broken LC_ALL wins even though LANG alone would have been fine.
    let vars = |k: &str| match k {
        "LC_ALL" => Some("C".to_string()),
        "LANG" => Some("en_US.UTF-8".to_string()),
        _ => None,
    };
    assert!(needs_default_locale(vars));

    // LC_CTYPE is used when LC_ALL isn't set, ahead of LANG.
    let vars = |k: &str| match k {
        "LC_CTYPE" => Some("en_US.UTF-8".to_string()),
        "LANG" => Some("C".to_string()),
        _ => None,
    };
    assert!(!needs_default_locale(vars));
}

#[test]
fn env_sets_terminal_type_and_removes_multiplexer_and_size_markers() {
    let mut builder = CommandBuilder::new("/bin/true");
    builder.env("TMUX", "1");
    builder.env("ITERM_SESSION_ID", "w0t0p0");
    builder.env("COLUMNS", "80");
    builder.env("LINES", "24");
    configure_env(&mut builder);
    assert_eq!(
        builder.get_env("TERM"),
        Some(std::ffi::OsStr::new("xterm-256color"))
    );
    assert_eq!(
        builder.get_env("COLORTERM"),
        Some(std::ffi::OsStr::new("truecolor"))
    );
    assert!(builder.get_env("TMUX").is_none());
    assert!(builder.get_env("ITERM_SESSION_ID").is_none());
    assert!(builder.get_env("COLUMNS").is_none());
    assert!(builder.get_env("LINES").is_none());
}

#[test]
fn cwd_uses_the_given_dir_when_it_exists() {
    let tmp = std::env::temp_dir();
    assert_eq!(resolve_cwd(tmp.to_str()), tmp);
}

#[test]
fn cwd_falls_back_to_home_when_missing_or_not_a_directory() {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    assert_eq!(resolve_cwd(Some("/no/such/dir-nodal-test")), home);
    assert_eq!(resolve_cwd(None), home);
}

#[test]
fn generation_starts_at_zero_and_close_owned_by_bumps_it() {
    let sessions = PtySessions::default();
    assert_eq!(sessions.generation_for("main"), 0);
    sessions.close_owned_by("main");
    assert_eq!(sessions.generation_for("main"), 1);
    // Unaffected: a different webview label has its own counter.
    assert_eq!(sessions.generation_for("other"), 0);
}
