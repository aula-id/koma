use super::is_stall;

#[test]
fn empty_is_stall() {
    assert!(is_stall(""));
    assert!(is_stall("   "));
}

#[test]
fn colon_cliffhanger_is_stall() {
    assert!(is_stall("Let me read more:"));
}

#[test]
fn lead_in_phrase_is_stall() {
    assert!(is_stall("I'll check"));
}

#[test]
fn substantial_multi_line_is_not_stall() {
    assert!(!is_stall("Here is the answer.\nIt spans two lines."));
}

#[test]
fn substantial_long_is_not_stall() {
    let long = "a".repeat(300);
    assert!(!is_stall(&long));
}

#[test]
fn list_body_is_not_stall() {
    assert!(!is_stall("Findings:\n- item one"));
}

#[test]
fn heading_body_is_not_stall() {
    assert!(!is_stall("## Summary\nDone."));
}

#[test]
fn table_body_is_not_stall() {
    assert!(!is_stall("| col | val |"));
}
