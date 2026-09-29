use super::*;

#[test]
fn agents_json_filters_background_and_tolerates_missing_fields() {
    let text = r#"[
          {"id":"0eef7f11","cwd":"/a","kind":"background","startedAt":1787690543980,
           "sessionId":"0eef7f11-d932-4001-97f8-df01555801d7","name":"old","state":"done"},
          {"pid":59575,"cwd":"/b","kind":"interactive","startedAt":1790087661575,
           "sessionId":"a5ff54f0-6f8e-49b4-9213-0c859d3432de","name":"interactive","status":"idle"},
          {"pid":123,"id":"ddb91222","cwd":"/c","kind":"background","startedAt":1790192548000,
           "sessionId":"ddb91222-57b2-4ae4-a0bb-d1c5993dc1c8","name":"new","status":"busy",
           "state":"working","newField":{"x":1}},
          {"id":"nosession","kind":"background"},
          {"id":"odd","sessionId":"s","kind":"background","pid":"not-a-number","startedAt":"yesterday"}
        ]"#;
    let runs = parse_agents_json(text).unwrap();
    let ids: Vec<&str> = runs.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["ddb91222", "0eef7f11", "odd"]);
    assert_eq!(runs[0].pid, Some(123));
    assert_eq!(runs[0].status.as_deref(), Some("busy"));
    assert_eq!(runs[0].state.as_deref(), Some("working"));
    assert_eq!(runs[1].pid, None);
    assert_eq!(runs[1].status, None);
    assert_eq!(runs[2].pid, None);
    assert_eq!(runs[2].started_at, None);
    assert!(parse_agents_json("no json").is_err());
}

#[test]
fn agents_json_exposes_blocked_on_permission() {
    // Real entry from a --bg session waiting for approval of a Write (claude 2.1.281).
    let text = r#"[{"pid":21111,"id":"af5deb85","cwd":"/Users/me/Code/nodal-sandbox",
          "kind":"background","startedAt":1790198688952,"sessionId":"af5deb85-3fe1-4ea9-b172-00b68389e167",
          "name":"create perm-test.txt","status":"waiting","waitingFor":"permission prompt","state":"blocked"}]"#;
    let r = &parse_agents_json(text).unwrap()[0];
    assert_eq!(r.state.as_deref(), Some("blocked"));
    assert_eq!(r.status.as_deref(), Some("waiting"));
    assert_eq!(r.waiting_for.as_deref(), Some("permission prompt"));
    assert!(r.is_in_progress());
}
