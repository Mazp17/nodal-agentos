use super::*;

use tokio::io::AsyncWriteExt;

/// P08, offline: `forward_lines` forwards only the first `MAX_LAUNCH_OUTPUT_LINES` (the
/// `cfg(test)` value, 5) and still drains the rest instead of leaving it unread (the writer
/// finishes without blocking on a full pipe).
#[test]
fn forward_lines_caps_what_it_forwards_but_drains_the_rest() {
    crate::testutil::block_on(async {
        let (mut writer, reader) = tokio::io::duplex(64 * 1024);
        let (tx, mut rx) = mpsc::unbounded_channel();
        forward_lines(reader, tx, &Handle::current());

        let total = MAX_LAUNCH_OUTPUT_LINES * 3;
        let mut sent = String::new();
        for i in 0..total {
            sent.push_str(&format!("line {i}\n"));
        }
        tokio::time::timeout(Duration::from_secs(5), writer.write_all(sent.as_bytes()))
            .await
            .expect("writer blocked: the reader isn't draining past the cap")
            .unwrap();
        drop(writer);

        let mut forwarded = Vec::new();
        while let Some(line) = rx.recv().await {
            forwarded.push(line);
        }
        assert_eq!(forwarded.len() as u64, MAX_LAUNCH_OUTPUT_LINES);
        assert_eq!(forwarded[0], "line 0");
    });
}

/// Against the real `claude` and this machine's data: `cargo test -- --ignored`.
/// Read-only (`claude agents` + files); doesn't launch runs.
#[test]
#[ignore]
fn real_list_and_detail() {
    crate::testutil::block_on(async {
        let runs = list_runs(&AgentsRaw::new()).await.expect("list_runs");
        eprintln!("{} background runs", runs.len());
        for r in runs.iter().take(5) {
            let cwd = r.cwd.clone().unwrap_or_default();
            let d = crate::claude::fs::readout::read_session(&r.session_id, &cwd);
            eprintln!(
                "{} {:?} {:?} -> {:?}",
                r.id,
                r.state,
                r.name,
                d.detail
                    .map(|d| (d.workflow_id, d.source, d.current_phase, d.agents.len()))
            );
        }
        let rt = Handle::current();
        let err = launch_bg(
            "/no/such/dir".into(),
            "x".into(),
            &LaunchOptions::default(),
            &ExtraFlags::default(),
            &rt,
        )
        .await
        .unwrap_err();
        assert_eq!(
            err.message,
            "The folder doesn't exist or isn't a directory: /no/such/dir"
        );
    });
}

/// Against the real `claude`: `cargo test -- --ignored`. Launches two runs (plain and with
/// `--agent code-reviewer`, which must exist in `~/.claude/agents`) and stops them.
#[test]
#[ignore]
fn real_launch_with_append_system_prompt() {
    let cwd = env!("CARGO_MANIFEST_DIR").to_string();
    let prompt = "Reply with the single word OK.".to_string();
    let sp = Some(nodal_domain::execution::prompts::UNATTENDED_SYSTEM_PROMPT.to_string());
    crate::testutil::block_on(async {
        let rt = Handle::current();
        for agent in [None, Some("code-reviewer".to_string())] {
            let extra = ExtraFlags {
                agent: agent.clone(),
                append_system_prompt: sp.clone(),
                ..Default::default()
            };
            let run = launch_bg(
                cwd.clone(),
                prompt.clone(),
                &LaunchOptions::default(),
                &extra,
                &rt,
            )
            .await
            .unwrap_or_else(|e| panic!("launch with agent {agent:?}: {}", e.message));
            eprintln!("agent {agent:?} -> backgrounded · {}", run.id);
            stop(&run.id).await.expect("stop");
        }
    });
}

/// P08 before/after: a `claude --bg` that floods its output while `launch_bg` hasn't drained
/// the channel yet. Lines and bytes left queued in the unbounded channel = what the flood costs
/// in memory. `cargo test -p nodal-host --lib measure_forward_lines -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_forward_lines_queued_under_a_flood() {
    use tokio::io::AsyncWriteExt;
    const LINES: usize = 20_000;
    crate::testutil::block_on(async {
        let (mut writer, reader) = tokio::io::duplex(64 * 1024);
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        forward_lines(reader, tx, &Handle::current());
        let line = format!("{}\n", "x".repeat(63));
        for _ in 0..LINES {
            writer.write_all(line.as_bytes()).await.unwrap();
        }
        drop(writer);
        let (mut queued, mut bytes) = (0usize, 0usize);
        while let Some(l) = rx.recv().await {
            queued += 1;
            bytes += l.len();
        }
        eprintln!("P08 forward_lines: {LINES} lines written -> {queued} lines / {bytes} B queued");
    });
}
