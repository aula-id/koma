use super::*;

fn p(lane: Lane) -> String {
    prompt_for(lane, "a.example:443")
}

#[test]
fn every_lane_names_itself() {
    assert!(p(Lane::Research).contains("research lane"));
    assert!(p(Lane::Red).contains("red lane"));
    assert!(p(Lane::Blue).contains("blue lane"));
}

#[test]
fn the_scope_line_is_carried() {
    for lane in [Lane::Research, Lane::Red, Lane::Blue] {
        assert!(p(lane).contains("a.example:443"));
    }
}

#[test]
fn the_shared_rules_reach_every_lane() {
    for lane in [Lane::Research, Lane::Red, Lane::Blue] {
        assert!(p(lane).contains("Untested is not the same as unproven"));
    }
}

#[test]
fn research_is_told_it_does_not_touch_a_target() {
    let text = p(Lane::Research);
    assert!(text.contains("do not touch a target"));
    assert!(text.contains("refused"));
}

#[test]
fn red_is_told_it_does_not_write() {
    assert!(p(Lane::Red).contains("do not write to the tree"));
}

#[test]
fn blue_is_told_it_proposes_and_must_re_verify() {
    let text = p(Lane::Blue);
    assert!(text.contains("propose a diff"));
    assert!(text.contains("only written"));
}

#[test]
fn the_prompts_are_not_identical() {
    assert_ne!(p(Lane::Red), p(Lane::Blue));
    assert_ne!(p(Lane::Research), p(Lane::Red));
}
