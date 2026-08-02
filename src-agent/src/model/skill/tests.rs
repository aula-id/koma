//! Unit tests for the skill module.

use super::def::SkillSource;
use super::parse::{load_skill_file, parse_skill, validate_skill_name};
use super::registry::SkillRegistry;
use std::collections::BTreeMap;

// ── Parse tests ────────────────────────────────────────────────────────────

#[test]
fn parses_full_frontmatter_and_body() {
    let content = "---\n\
        name: my-skill\n\
        description: When to use this skill for doing things.\n\
        ---\n\
        # My Skill\n\n\
        These are the instructions.";
    let s = parse_skill(
        "my-skill",
        content,
        SkillSource::Global,
        "/tmp/my-skill.md".into(),
        None,
    )
    .unwrap();
    assert_eq!(s.name, "my-skill");
    assert_eq!(s.description, "When to use this skill for doing things.");
    assert_eq!(s.body, "# My Skill\n\nThese are the instructions.");
    assert_eq!(s.source, SkillSource::Global);
}

#[test]
fn name_from_filename_when_frontmatter_omits_name() {
    // Filename stem is canonical; frontmatter name is ignored.
    let content = "---\ndescription: A useful skill.\n---\nBody here.";
    let s = parse_skill(
        "foo-bar",
        content,
        SkillSource::Claude,
        "/tmp/foo-bar.md".into(),
        None,
    )
    .unwrap();
    assert_eq!(s.name, "foo-bar");
    assert_eq!(s.description, "A useful skill.");
}

#[test]
fn missing_description_is_an_error() {
    let content = "---\nname: test\n---\nSome body.";
    let s = parse_skill(
        "test",
        content,
        SkillSource::Global,
        "/tmp/test.md".into(),
        None,
    );
    assert!(s.is_err());
    assert!(s
        .unwrap_err()
        .to_string()
        .contains("missing required 'description'"));
}

#[test]
fn unclosed_fence_is_an_error() {
    let content = "---\ndescription: oops\nno closing fence";
    let s = parse_skill(
        "oops",
        content,
        SkillSource::Global,
        "/tmp/oops.md".into(),
        None,
    );
    assert!(s.is_err());
}

#[test]
fn body_only_no_frontmatter_is_an_error() {
    // No frontmatter means no description → error.
    let content = "Just instructions, no frontmatter.";
    let s = parse_skill(
        "bare",
        content,
        SkillSource::Global,
        "/tmp/bare.md".into(),
        None,
    );
    assert!(s.is_err());
}

#[test]
fn empty_body_allowed_with_description() {
    let content = "---\ndescription: Has desc, no body.\n---\n";
    let s = parse_skill(
        "has-desc",
        content,
        SkillSource::Global,
        "/tmp/has-desc.md".into(),
        None,
    )
    .unwrap();
    assert!(s.body.is_empty());
}

#[test]
fn extra_unknown_yaml_keys_tolerated() {
    let content = "---\n\
        description: Tolerant skill.\n\
        license: MIT\n\
        compatibility: \">= 1.0\"\n\
        metadata:\n\
          author: test\n\
        ---\n\
        Body.";
    let s = parse_skill(
        "tolerant",
        content,
        SkillSource::Global,
        "/tmp/tolerant.md".into(),
        None,
    )
    .unwrap();
    assert_eq!(s.description, "Tolerant skill.");
}

#[test]
fn skill_dir_set_for_dir_form() {
    let content = "---\ndescription: Dir skill.\n---\nBody.";
    let s = parse_skill(
        "dskill",
        content,
        SkillSource::ProjectAgents,
        "/tmp/skills/dskill/SKILL.md".into(),
        Some("/tmp/skills/dskill".into()),
    )
    .unwrap();
    assert_eq!(
        s.skill_dir,
        Some(std::path::PathBuf::from("/tmp/skills/dskill"))
    );
}

// ── Name validation tests ──────────────────────────────────────────────────

#[test]
fn valid_skill_names() {
    assert_eq!(validate_skill_name("foo").unwrap(), "foo");
    assert_eq!(validate_skill_name("my-skill").unwrap(), "my-skill");
    assert_eq!(validate_skill_name("PINEAPPLE").unwrap(), "pineapple");
    assert_eq!(validate_skill_name("My-Agent").unwrap(), "my-agent");
}

#[test]
fn invalid_skill_names() {
    assert!(validate_skill_name("").is_err());
    assert!(validate_skill_name("-x").is_err());
    assert!(validate_skill_name("x-").is_err());
    assert!(validate_skill_name("a/b").is_err());
    assert!(validate_skill_name("..").is_err());
    assert!(validate_skill_name("../x").is_err());
    assert!(validate_skill_name("a b").is_err());
    assert!(validate_skill_name("a.b").is_err());
    assert!(validate_skill_name("Foo_Bar").is_err());
}

// ── Registry discovery tests ──────────────────────────────────────────────

/// Helper: create a temp skill root with the given file/dir entries.
fn setup_skill_root(
    base: &std::path::Path,
    entries: &[(&str, &str)], // (filename_or_dir, content)
) {
    std::fs::create_dir_all(base).unwrap();
    for (name, content) in entries {
        if name.contains('/') {
            // Dir form: "dir/entry_file"
            let parts: Vec<&str> = name.splitn(2, '/').collect();
            let dir = base.join(parts[0]);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(parts[1]), content).unwrap();
        } else {
            std::fs::write(base.join(name), content).unwrap();
        }
    }
}

#[test]
fn empty_roots_gives_empty_registry() {
    let reg = SkillRegistry::load(None);
    assert!(reg.is_empty());
    assert!(reg.catalogue_text().is_empty());
}

#[test]
fn flat_skill_discovered() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-test-{}",
        uuid::Uuid::new_v4()
    ));
    let root = tmp.join("skills");
    setup_skill_root(
        &root,
        &[("foo.md", "---\ndescription: Foo skill.\n---\nBody.")],
    );

    // Manually scan the root (not using full load, which hits real ~/.koma).
    let mut skills = BTreeMap::new();
    super::registry::scan_skills_root_for_test(&root, SkillSource::Global, &mut skills);
    assert!(skills.contains_key("foo"));
    assert_eq!(skills["foo"].description, "Foo skill.");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn dir_form_skill_discovered() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-dir-{}",
        uuid::Uuid::new_v4()
    ));
    let root = tmp.join("skills");
    setup_skill_root(
        &root,
        &[("bar/SKILL.md", "---\ndescription: Bar skill.\n---\nBar body.")],
    );

    let mut skills = BTreeMap::new();
    super::registry::scan_skills_root_for_test(&root, SkillSource::Global, &mut skills);
    assert!(skills.contains_key("bar"));
    assert_eq!(skills["bar"].description, "Bar skill.");
    assert!(skills["bar"].skill_dir.is_some());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn readme_md_skipped() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-readme-{}",
        uuid::Uuid::new_v4()
    ));
    let root = tmp.join("skills");
    setup_skill_root(
        &root,
        &[
            ("README.md", "# Skills\nHere are skills."),
            ("real.md", "---\ndescription: Real.\n---\nBody."),
        ],
    );

    let mut skills = BTreeMap::new();
    super::registry::scan_skills_root_for_test(&root, SkillSource::Global, &mut skills);
    assert!(!skills.contains_key("readme"));
    assert!(skills.contains_key("real"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn malformed_file_skipped_siblings_still_load() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-mal-{}",
        uuid::Uuid::new_v4()
    ));
    let root = tmp.join("skills");
    setup_skill_root(
        &root,
        &[
            ("bad.md", "---\nno desc\n---\nBody."), // no description
            ("good.md", "---\ndescription: Good.\n---\nOk."),
        ],
    );

    let mut skills = BTreeMap::new();
    super::registry::scan_skills_root_for_test(&root, SkillSource::Global, &mut skills);
    assert!(!skills.contains_key("bad"));
    assert!(skills.contains_key("good"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn lowercase_skill_md_accepted() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-lc-{}",
        uuid::Uuid::new_v4()
    ));
    let root = tmp.join("skills");
    setup_skill_root(
        &root,
        &[("mydir/skill.md", "---\ndescription: Lowercase entry.\n---\nBody.")],
    );

    let mut skills = BTreeMap::new();
    super::registry::scan_skills_root_for_test(&root, SkillSource::Global, &mut skills);
    assert!(skills.contains_key("mydir"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn non_md_files_ignored() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-nonmd-{}",
        uuid::Uuid::new_v4()
    ));
    let root = tmp.join("skills");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("notes.txt"), "not a skill").unwrap();
    std::fs::write(root.join("data.json"), "{}").unwrap();

    let mut skills = BTreeMap::new();
    super::registry::scan_skills_root_for_test(&root, SkillSource::Global, &mut skills);
    assert!(skills.is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn nested_junk_ignored() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-nested-{}",
        uuid::Uuid::new_v4()
    ));
    let root = tmp.join("skills");
    // Create a dir without SKILL.md — should not be loaded.
    let nested = root.join("notaskill");
    std::fs::create_dir_all(nested.join("sub")).unwrap();
    std::fs::write(nested.join("readme.md"), "hi").unwrap();

    let mut skills = BTreeMap::new();
    super::registry::scan_skills_root_for_test(&root, SkillSource::Global, &mut skills);
    assert!(skills.is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn duplicate_name_last_wins() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-dup-{}",
        uuid::Uuid::new_v4()
    ));
    let root = tmp.join("skills");
    setup_skill_root(
        &root,
        &[
            ("alpha.md", "---\ndescription: First.\n---\nV1."),
            ("beta/SKILL.md", "---\ndescription: Beta.\n---\nBody."),
        ],
    );

    let mut skills = BTreeMap::new();
    super::registry::scan_skills_root_for_test(&root, SkillSource::Global, &mut skills);
    // Both should be present; they have different names.
    assert_eq!(skills.len(), 2);
    assert!(skills.contains_key("alpha"));
    assert!(skills.contains_key("beta"));

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── Catalogue text tests ──────────────────────────────────────────────────

#[test]
fn catalogue_sorted_by_name() {
    // Build two SkillDefs via parse and verify their names.
    let s1 = parse_skill(
        "zebra",
        "---\ndescription: Z skill.\n---\n",
        SkillSource::Global,
        "/tmp/zebra.md".into(),
        None,
    )
    .unwrap();
    let s2 = parse_skill(
        "alpha",
        "---\ndescription: A skill.\n---\n",
        SkillSource::Global,
        "/tmp/alpha.md".into(),
        None,
    )
    .unwrap();
    assert_eq!(s1.name, "zebra");
    assert_eq!(s2.name, "alpha");
}

#[test]
fn catalogue_text_format() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-cat-{}",
        uuid::Uuid::new_v4()
    ));
    let root = tmp.join("skills");
    setup_skill_root(
        &root,
        &[
            ("zebra.md", "---\ndescription: Zebra skill.\n---\n"),
            ("alpha.md", "---\ndescription: Alpha skill.\n---\n"),
        ],
    );

    let mut skills = BTreeMap::new();
    super::registry::scan_skills_root_for_test(&root, SkillSource::Global, &mut skills);

    // BTreeMap gives sorted-by-key iteration.
    let mut names: Vec<&str> = skills.keys().map(|s| s.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["alpha", "zebra"]);

    let catalogue: Vec<String> = skills
        .values()
        .map(|s| format!("- {}: {}", s.name, s.description))
        .collect();
    assert!(catalogue[0].contains("alpha"));
    assert!(catalogue[1].contains("zebra"));

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── load_skill_file integration tests ────────────────────────────────────

#[test]
fn load_skill_file_flat() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-loadflat-{}",
        uuid::Uuid::new_v4()
    ));
    let path = tmp.join("foo.md");
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(&path, "---\ndescription: Flat load.\n---\nBody content.")
        .unwrap();

    let skill = load_skill_file(&path, SkillSource::ProjectAgent, None).unwrap();
    assert_eq!(skill.name, "foo");
    assert_eq!(skill.description, "Flat load.");
    assert_eq!(skill.body, "Body content.");
    assert_eq!(skill.source, SkillSource::ProjectAgent);
    assert!(skill.skill_dir.is_none());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn load_skill_file_with_skill_dir() {
    let tmp = std::env::temp_dir().join(format!(
        "koma-skill-loaddir-{}",
        uuid::Uuid::new_v4()
    ));
    let skill_root = tmp.join("bar");
    let path = skill_root.join("SKILL.md");
    std::fs::create_dir_all(&skill_root).unwrap();
    std::fs::write(&path, "---\ndescription: Dir load.\n---\nDir body.")
        .unwrap();

    let skill =
        load_skill_file(&path, SkillSource::Claude, Some(skill_root.clone())).unwrap();
    assert_eq!(skill.name, "bar");
    assert_eq!(skill.description, "Dir load.");
    assert_eq!(skill.body, "Dir body.");
    assert_eq!(skill.skill_dir, Some(skill_root));

    let _ = std::fs::remove_dir_all(&tmp);
}
