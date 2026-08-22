#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::append_active_skills;
use crate::app::state::ActiveSkill;
use std::collections::BTreeMap;

#[test]
fn append_skills_is_ordered_and_skips_blank() {
    let mut skills = BTreeMap::new();
    skills.insert(
        "zebra".to_string(),
        ActiveSkill {
            body: "z-body".to_string(),
            skill_dir: None,
        },
    );
    skills.insert(
        "alpha".to_string(),
        ActiveSkill {
            body: "a-body".to_string(),
            skill_dir: None,
        },
    );
    skills.insert(
        "blank".to_string(),
        ActiveSkill {
            body: "   ".to_string(),
            skill_dir: None,
        },
    );
    let mut dst = String::from("HEAD");
    append_active_skills(&mut dst, &skills);
    assert_eq!(
        dst,
        "HEAD\n\n# Skill: alpha\na-body\n\n# Skill: zebra\nz-body"
    );
}

#[test]
fn empty_map_noop() {
    let skills = BTreeMap::new();
    let mut dst = String::from("HEAD");
    append_active_skills(&mut dst, &skills);
    assert_eq!(dst, "HEAD");
}
