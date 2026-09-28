use super::*;

#[test]
fn script_phases_best_effort() {
    let src = "export const meta = { name: 'x', phases: [ { title: 'One', detail: \"with } brace\" },\n { title: `Two` } ], }";
    let phases = parse_script_phases(src);
    assert_eq!(phases.len(), 2);
    assert_eq!(phases[0].detail.as_deref(), Some("with } brace"));
    assert_eq!(phases[1].title, "Two");
    assert!(parse_script_phases("no phases").is_empty());
}
