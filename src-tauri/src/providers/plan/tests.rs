use super::*;
use crate::domain::{ExternalState, Priority, ScopeRef};
use crate::providers::{ChildItem, ItemRef};

fn state(id: &str, name: &str, kind: ExtKind) -> ExternalState {
    ExternalState {
        id: id.into(),
        name: name.into(),
        kind,
        color: None,
    }
}

pub fn item(n: u32, desc: Option<&str>) -> ExternalItem {
    ExternalItem {
        external_id: format!("uuid-{n}"),
        identifier: format!("ENG-{n}"),
        url: format!("https://linear.app/acme/issue/ENG-{n}/x"),
        title: format!("Issue {n}"),
        description_md: desc.map(Into::into),
        state: state("s-todo", "Todo", ExtKind::Unstarted),
        scopes: vec![ScopeRef {
            kind: "team".into(),
            id: "team-eng".into(),
            name: "Engineering".into(),
        }],
        parent: None,
        children: Vec::new(),
        labels: Vec::new(),
        assignee: None,
        priority: Priority::None,
        updated_at: "2026-09-20T10:00:00.000Z".into(),
        created_at: Some("2026-09-01T10:00:00.000Z".into()),
        closed_at: None,
    }
}

#[test]
fn plan_snapshot() {
    let mut it = item(
        142,
        Some("Change the site logo.\n\n## Acceptance criteria\n- New logo in the header\n"),
    );
    it.title = "Website rebrand".into();
    it.parent = Some(ItemRef {
        external_id: "uuid-100".into(),
        identifier: "ENG-100".into(),
        title: "New brand".into(),
        url: "https://linear.app/acme/issue/ENG-100/new-brand".into(),
    });
    it.children = vec![
        ChildItem {
            external_id: "uuid-143".into(),
            identifier: "ENG-143".into(),
            title: "Header".into(),
            url: "https://linear.app/acme/issue/ENG-143/header".into(),
            state: state("s-done", "Done", ExtKind::Completed),
        },
        ChildItem {
            external_id: "uuid-144".into(),
            identifier: "ENG-144".into(),
            title: "Favicon".into(),
            url: "https://linear.app/acme/issue/ENG-144/favicon".into(),
            state: state("s-todo", "Todo", ExtKind::Unstarted),
        },
    ];
    let expected = "\
# ENG-142 · Website rebrand

https://linear.app/acme/issue/ENG-142/x

Parent: [ENG-100](https://linear.app/acme/issue/ENG-100/new-brand) New brand

Change the site logo.

## Acceptance criteria
- New logo in the header

## Subtasks

- [x] [ENG-143](https://linear.app/acme/issue/ENG-143/header) Header (Done)
- [ ] [ENG-144](https://linear.app/acme/issue/ENG-144/favicon) Favicon (Todo)

---

Do not update the task manager: Nodal syncs the status.
";
    assert_eq!(render_plan(&it), expected);
}

#[test]
fn plan_without_description_or_children() {
    let p = render_plan(&item(7, None));
    assert!(p.contains("_No description._"));
    assert!(!p.contains("## Subtasks"));
    assert!(p.trim_end().ends_with(FOOTER));
}

#[test]
fn write_plan_only_when_changed() {
    let dir = std::env::temp_dir().join(format!("nodal-plan-{}", crate::util::new_id('x', 1)));
    let path = plan_path(&dir, "t1");
    assert!(write_plan(&path, "a").unwrap());
    assert!(!write_plan(&path, "a").unwrap());
    assert!(write_plan(&path, "b").unwrap());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "b");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn extracts_criteria_from_heading_section() {
    let md = "Intro.\n\n## Acceptance Criteria\n\nThe feature is done when:\n\n- [ ] Logo in the header\n- [x] Favicon\n  with transparent background\n1. Green tests\n\n## Notes\n- not a criterion\n";
    assert_eq!(
        extract_acceptance(md),
        vec![
            "Logo in the header",
            "Favicon with transparent background",
            "Green tests"
        ]
    );
}

#[test]
fn extracts_criteria_from_bold_or_colon_titles() {
    let md = "**Criterios de aceptación**\n* One\n* Two\n\nAnother paragraph.\n- three\n";
    assert_eq!(extract_acceptance(md), vec!["One", "Two"]);
    let md = "Context.\n\nDone when:\n- Deploy ok\n- Docs\n";
    assert_eq!(extract_acceptance(md), vec!["Deploy ok", "Docs"]);
    let md = "### Criterios\n- a\n#### Sub\n- b\n## End\n- c";
    assert_eq!(extract_acceptance(md), vec!["a"]);
}

#[test]
fn no_section_no_criteria() {
    assert!(extract_acceptance("Just text.\n- some random list\n").is_empty());
    assert!(extract_acceptance("").is_empty());
    assert!(extract_acceptance("## Acceptance\n\nNothing in a list.").is_empty());
}

#[test]
fn closing_comment_with_verdict() {
    let v = Verdict {
        pass: false,
        unmet: vec!["Favicon updated".into()],
        nits: vec!["Rename variable".into()],
        summary: Some("Favicon is missing.".into()),
    };
    let c = closing_comment(&ClosingInfo {
        status: Some(TaskStatus::Blocked),
        executor: Some("frontend-developer"),
        branch: Some("nodal/eng-142"),
        verdict: Some(&v),
        ..Default::default()
    });
    assert_eq!(
        c,
        "**Nodal** · Blocked · frontend-developer\n\nBranch: `nodal/eng-142`\n\nFavicon is missing.\n\n**Unmet criteria**\n- Favicon updated\n\n**Nits**\n- Rename variable"
    );
    let c = closing_comment(&ClosingInfo {
        status: Some(TaskStatus::InReview),
        pr_url: Some("https://example.com/pr/1"),
        branch: Some("b"),
        note: Some("Finished without a report."),
        ..Default::default()
    });
    assert!(c.contains("PR: https://example.com/pr/1") && !c.contains("Branch"));
    assert!(c.ends_with("_Finished without a report._"));
}
