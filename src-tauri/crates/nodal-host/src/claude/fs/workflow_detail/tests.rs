use std::path::PathBuf;

use super::*;
use crate::claude::fs::paths::{claude_config_dir, find_session_dir, project_slug};

const SANDBOX: &str = "/Users/me/Code/nodal-sandbox";
const DONE_SESSION: &str = "ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8";
const CUT_SESSION: &str = "91a57808-74f6-40fd-9fbf-184b7358bbe4";

fn fixtures_projects() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/claude/fixtures/projects")
}

#[test]
fn completed_run_uses_final_summary() {
    let dir = fixtures_projects()
        .join(project_slug(SANDBOX))
        .join(DONE_SESSION);
    let d = read_run_detail(&dir).unwrap();
    assert_eq!(d.source, DetailSource::Final);
    assert_eq!(d.workflow_id, "wf_2450b7a8-254");
    assert_eq!(d.workflow_name.as_deref(), Some("demo-board"));
    assert_eq!(d.status.as_deref(), Some("completed"));
    assert_eq!(d.result_status.as_deref(), Some("green"));
    let titles: Vec<&str> = d.phases.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(titles, ["Idear", "Escribir", "Revisar", "Cerrar"]);
    assert_eq!(
        d.phases[0].detail.as_deref(),
        Some("un agente propone 4 subtemas")
    );
    assert_eq!(d.current_phase.as_deref(), Some("Cerrar"));
    assert_eq!(d.current_phase_index, Some(4));
    assert_eq!(d.agent_count, 10);
    assert_eq!(d.agents.len(), 10);
    assert!(d.agents.iter().all(|a| a.state == AgentState::Done));
    assert_eq!(d.total_tokens, Some(377324));
    assert_eq!(d.total_tool_calls, Some(45));
    assert_eq!(d.duration_ms, Some(117897));
    assert_eq!(d.workflow_count, 1);
    let first = &d.agents[0];
    assert_eq!(first.label, "idear:volcanes");
    assert_eq!(first.agent_id.as_deref(), Some("a6daf84e776bfaad5"));
    assert_eq!(first.model.as_deref(), Some("claude-sonnet-5"));
    assert_eq!(first.phase.as_deref(), Some("Idear"));
    assert_eq!(first.tokens, Some(34612));
    assert_eq!(first.tool_calls, Some(1));
    let last = d.agents.last().unwrap();
    assert_eq!(last.last_tool_name.as_deref(), Some("Bash"));
    assert_eq!(
        last.last_tool_summary.as_deref(),
        Some("ls -la ./demo-out && wc -w ./demo-out/*.md")
    );
}

#[test]
fn cut_run_is_rebuilt_from_journal() {
    let dir = fixtures_projects()
        .join(project_slug(SANDBOX))
        .join(CUT_SESSION);
    let d = read_run_detail(&dir).unwrap();
    assert_eq!(d.source, DetailSource::Live);
    assert_eq!(d.workflow_id, "wf_4ec5fcf1-d6b");
    assert_eq!(d.workflow_name.as_deref(), Some("demo-board"));
    assert_eq!(d.status, None);
    assert_eq!(d.result_status, None);
    // Phases from the script's meta: the total is known even if they weren't reached.
    let titles: Vec<&str> = d.phases.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(titles, ["Idear", "Escribir", "Revisar", "Cerrar"]);
    assert_eq!(d.current_phase.as_deref(), Some("Escribir"));
    assert_eq!(d.current_phase_index, Some(2));
    assert_eq!(d.agent_count, 5);
    let states: Vec<(&str, AgentState)> = d
        .agents
        .iter()
        .map(|a| (a.label.as_str(), a.state))
        .collect();
    assert_eq!(
        states,
        [
            ("idear:volcanes", AgentState::Done),
            ("escribir:formacion-volcanica", AgentState::Running),
            ("escribir:tipos-de-volcanes", AgentState::Running),
            ("escribir:erupciones-volcanicas", AgentState::Running),
            ("escribir:volcanes-y-el-planeta", AgentState::Running),
        ]
    );
    assert!(d
        .agents
        .iter()
        .all(|a| a.model.as_deref() == Some("claude-haiku-4-5-20251001")));
    // Last tool taken from the transcript's tail (only agents in progress).
    let planeta = d
        .agents
        .iter()
        .find(|a| a.label == "escribir:volcanes-y-el-planeta")
        .unwrap();
    assert_eq!(planeta.last_tool_name.as_deref(), Some("Bash"));
    assert_eq!(planeta.last_tool_summary.as_deref(), Some("sleep 34"));
    let idear = &d.agents[0];
    assert_eq!(idear.last_tool_name, None);
}

#[test]
fn journal_ignores_unknown_types_and_broken_lines() {
    let text = concat!(
        "{\"type\":\"launched\"}\n",
        "{\"type\":\"started\",\"key\":\"k1\",\"agentId\":\"a1\",\"label\":\"one\",\"phase\":\"A\"}\n",
        "{\"type\":\"something-new\",\"agentId\":\"a1\"}\n",
        "{\"type\":\"started\",\"key\":\"k2\",\"label\":\"two\",\"phase\":\"B\"}\n",
        "{\"type\":\"result\",\"key\":\"k2\",\"result\":null}\n",
        "{\"type\":\"started\",\"key\":\"k3\",\"agentId\":\"a3\",\"lab",
    );
    let agents = parse_journal(text);
    assert_eq!(agents.len(), 2);
    assert_eq!(agents[0].state, AgentState::Running);
    assert_eq!(agents[1].label, "two");
    assert_eq!(agents[1].state, AgentState::Done);
}

#[test]
fn final_summary_tolerates_garbage() {
    assert!(parse_final("{\"runId\": \"wf_x\", \"workflowProg", "wf_x").is_none());
    let d = parse_final(r#"{"workflowProgress":[{"type":"workflow_phase","index":1,"title":"Only"},
            {"type":"workflow_agent","label":"x","phaseTitle":"Only","state":"exploded","tokens":"lots"}],
            "result":{"status":"purple"}}"#, "wf_y").unwrap();
    assert_eq!(d.workflow_id, "wf_y");
    assert_eq!(d.phases[0].title, "Only");
    assert_eq!(d.agents[0].state, AgentState::Unknown);
    assert_eq!(d.agents[0].tokens, None);
    assert_eq!(d.result_status, None);
}

#[test]
fn linear_issue_result_is_typed() {
    let v: Value = serde_json::from_str(
        r#"{"issue":"ACME-4","status":"green","pr":"https://github.com/o/r/pull/3","branch":"feat/acme-4",
            "workdir":"/w/run-acme-4","states":{"inReview":"In Review"},"children":["ACME-35"],"levels":[["ACME-35"]],
            "plan":null,"unmetAcceptance":["Works offline", 3],"nits":["Extract hook"," "]}"#,
    )
    .unwrap();
    let r = parse_result(Some(&v)).unwrap();
    assert_eq!(r.issue.as_deref(), Some("ACME-4"));
    assert_eq!(r.pr.as_deref(), Some("https://github.com/o/r/pull/3"));
    assert_eq!(r.branch.as_deref(), Some("feat/acme-4"));
    assert_eq!(r.workdir.as_deref(), Some("/w/run-acme-4"));
    assert_eq!(r.unmet_acceptance, Some(vec!["Works offline".to_string()]));
    assert_eq!(r.nits, Some(vec!["Extract hook".to_string()]));
    assert!(r.raw.unwrap().contains("\"children\""));

    // Another workflow: no known fields, but with the raw JSON.
    let demo = read_run_detail(
        &fixtures_projects()
            .join(project_slug(SANDBOX))
            .join(DONE_SESSION),
    )
    .unwrap();
    let res = demo.result.unwrap();
    assert_eq!((res.pr, res.unmet_acceptance, res.nits), (None, None, None));
    assert!(res.raw.unwrap().contains("\"tema\": \"volcanes\""));

    // PR that isn't a web URL: dropped. `null` or missing: no result.
    let v: Value =
        serde_json::from_str(r#"{"pr":"javascript:alert(1)","nits":"not-a-list"}"#).unwrap();
    let r = parse_result(Some(&v)).unwrap();
    assert_eq!((r.pr, r.nits), (None, None));
    assert_eq!(parse_result(Some(&Value::Null)), None);
    assert_eq!(parse_result(None), None);
    assert_eq!(
        parse_result(Some(&Value::String("done".into())))
            .unwrap()
            .raw
            .as_deref(),
        Some("done")
    );
}

#[test]
fn session_without_workflows_has_no_detail() {
    let tmp = std::env::temp_dir().join(format!("nodal-runs-test-{}", std::process::id()));
    fs::create_dir_all(&tmp).unwrap();
    assert_eq!(read_run_detail(&tmp), None);
    let _ = fs::remove_dir_all(&tmp);
}

/// Against this machine's real data:
/// `NODAL_SANDBOX=<sandbox repo path> cargo test -- --ignored`.
#[test]
#[ignore]
fn real_sessions_on_disk() {
    let sandbox = std::env::var("NODAL_SANDBOX").expect("NODAL_SANDBOX=<sandbox repo path>");
    let projects = claude_config_dir().unwrap().join("projects");
    for session in [DONE_SESSION, CUT_SESSION] {
        let dir = find_session_dir(&projects, &sandbox, session).expect("real session not found");
        let d = read_run_detail(&dir).expect("no workflow");
        eprintln!("{session}: {d:#?}");
    }
}
