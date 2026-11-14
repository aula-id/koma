//! Unit tests for the skill module.

use super::def::{SkillScopeWire, SkillSource};
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
    let reg = SkillRegistry::load_isolated(None, &[]);
    assert!(reg.is_empty());
    assert!(reg.prompt_text().is_none());
}

#[test]
fn flat_skill_discovered() {
    let tmp = std::env::temp_dir().join(format!("koma-skill-test-{}", uuid::Uuid::new_v4()));
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
    let tmp = std::env::temp_dir().join(format!("koma-skill-dir-{}", uuid::Uuid::new_v4()));
    let root = tmp.join("skills");
    setup_skill_root(
        &root,
        &[(
            "bar/SKILL.md",
            "---\ndescription: Bar skill.\n---\nBar body.",
        )],
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
    let tmp = std::env::temp_dir().join(format!("koma-skill-readme-{}", uuid::Uuid::new_v4()));
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
    let tmp = std::env::temp_dir().join(format!("koma-skill-mal-{}", uuid::Uuid::new_v4()));
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
    let tmp = std::env::temp_dir().join(format!("koma-skill-lc-{}", uuid::Uuid::new_v4()));
    let root = tmp.join("skills");
    setup_skill_root(
        &root,
        &[(
            "mydir/skill.md",
            "---\ndescription: Lowercase entry.\n---\nBody.",
        )],
    );

    let mut skills = BTreeMap::new();
    super::registry::scan_skills_root_for_test(&root, SkillSource::Global, &mut skills);
    assert!(skills.contains_key("mydir"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn non_md_files_ignored() {
    let tmp = std::env::temp_dir().join(format!("koma-skill-nonmd-{}", uuid::Uuid::new_v4()));
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
    let tmp = std::env::temp_dir().join(format!("koma-skill-nested-{}", uuid::Uuid::new_v4()));
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
    let tmp = std::env::temp_dir().join(format!("koma-skill-dup-{}", uuid::Uuid::new_v4()));
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
    let tmp = std::env::temp_dir().join(format!("koma-skill-cat-{}", uuid::Uuid::new_v4()));
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
    let tmp = std::env::temp_dir().join(format!("koma-skill-loadflat-{}", uuid::Uuid::new_v4()));
    let path = tmp.join("foo.md");
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(&path, "---\ndescription: Flat load.\n---\nBody content.").unwrap();

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
    let tmp = std::env::temp_dir().join(format!("koma-skill-loaddir-{}", uuid::Uuid::new_v4()));
    let skill_root = tmp.join("bar");
    let path = skill_root.join("SKILL.md");
    std::fs::create_dir_all(&skill_root).unwrap();
    std::fs::write(&path, "---\ndescription: Dir load.\n---\nDir body.").unwrap();

    let skill = load_skill_file(&path, SkillSource::Claude, Some(skill_root.clone())).unwrap();
    assert_eq!(skill.name, "bar");
    assert_eq!(skill.description, "Dir load.");
    assert_eq!(skill.body, "Dir body.");
    assert_eq!(skill.skill_dir, Some(skill_root));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn oversized_skill_entry_is_rejected_before_text_read() {
    let tmp = std::env::temp_dir().join(format!("koma-skill-oversized-{}", uuid::Uuid::new_v4()));
    let path = tmp.join("large.md");
    std::fs::create_dir_all(&tmp).unwrap();
    let mut bytes = b"---\ndescription: Large\n---\n".to_vec();
    bytes.resize(2 * 1024 * 1024 + 1, b'x');
    std::fs::write(&path, bytes).unwrap();
    assert!(load_skill_file(&path, SkillSource::Global, None)
        .unwrap_err()
        .to_string()
        .contains("2 MiB"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn parses_declared_metadata_and_preserves_unknown_frontmatter() {
    let content = "---\ndescription: Useful\ntriggers: when asked\nallowed-tools:\n  - Read\n  - Grep\nmetadata:\n  author: Ada\n---\nInstructions";
    let skill = parse_skill(
        "metadata",
        content,
        SkillSource::Global,
        "/tmp/metadata.md".into(),
        None,
    )
    .unwrap();

    assert_eq!(skill.triggers, "when asked");
    assert_eq!(skill.allowed_tools, vec!["Grep", "Read"]);
    assert!(skill
        .frontmatter
        .as_ref()
        .unwrap()
        .contains_key(serde_yaml_ng::Value::String("metadata".into())));
    assert!(skill
        .raw_frontmatter
        .as_deref()
        .unwrap()
        .contains("author: Ada"));
    assert!(skill.frontmatter_roundtrip_safe);
}

#[test]
fn advanced_yaml_is_discoverable_but_structured_save_is_fail_closed() {
    let content = "---\ndescription: Advanced\ndefaults: &defaults\n  owner: Ada\nmetadata: *defaults\n---\nBody";
    let skill = parse_skill(
        "advanced",
        content,
        SkillSource::Global,
        "/tmp/advanced.md".into(),
        None,
    )
    .unwrap();
    assert_eq!(skill.description, "Advanced");
    assert!(!skill.frontmatter_roundtrip_safe);
}

#[test]
fn source_identity_is_stable_but_generation_tracks_entry_bytes() {
    let first = parse_skill(
        "identity",
        "---\ndescription: One\n---\nBody",
        SkillSource::Global,
        "/tmp/identity.md".into(),
        None,
    )
    .unwrap();
    let second = parse_skill(
        "identity",
        "---\ndescription: Two\n---\nBody",
        SkillSource::Global,
        "/tmp/identity.md".into(),
        None,
    )
    .unwrap();
    assert_eq!(first.skill_id, second.skill_id);
    assert_ne!(first.generation, second.generation);
}

#[test]
fn catalogue_maps_all_v040_tiers_to_safe_ownership() {
    assert_eq!(SkillSource::Global.scope(), SkillScopeWire::Global);
    assert_eq!(SkillSource::ProjectAgent.scope(), SkillScopeWire::Project);
    assert_eq!(SkillSource::ProjectAgents.scope(), SkillScopeWire::Project);
    assert_eq!(SkillSource::Claude.scope(), SkillScopeWire::External);
    assert_eq!(SkillSource::ExtraRoot(4).scope(), SkillScopeWire::External);
    assert!(SkillSource::Global.editable());
    assert!(!SkillSource::Claude.editable());
}

#[test]
fn preserves_v040_tiers_and_external_root_wins_by_name() {
    let tmp = std::env::temp_dir().join(format!("koma-skill-all-tiers-{}", uuid::Uuid::new_v4()));
    let project = tmp.join("project");
    let external = tmp.join("external");
    for (relative, description) in [
        (".claude/skills/claude.md", "claude"),
        (".agent/skills/agent.md", "agent"),
        (".agents/skills/agents.md", "agents"),
        (".agents/skills/collision.md", "project"),
    ] {
        let path = project.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("---\ndescription: {description}\n---\nBody")).unwrap();
    }
    std::fs::create_dir_all(&external).unwrap();
    std::fs::write(
        external.join("collision.md"),
        "---\ndescription: external\n---\nBody",
    )
    .unwrap();

    let registry = SkillRegistry::load_isolated(Some(&project), std::slice::from_ref(&external));
    assert_eq!(registry.get("claude").unwrap().source, SkillSource::Claude);
    assert_eq!(
        registry.get("agent").unwrap().source,
        SkillSource::ProjectAgent
    );
    assert_eq!(
        registry.get("agents").unwrap().source,
        SkillSource::ProjectAgents
    );
    assert_eq!(
        registry.get("collision").unwrap().source,
        SkillSource::ExtraRoot(0)
    );
    assert_eq!(registry.catalogue().len(), 4);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn extra_roots_are_deduplicated_and_owned_overlap_is_rejected() {
    let tmp = std::env::temp_dir().join(format!("koma-skill-root-safety-{}", uuid::Uuid::new_v4()));
    let project = tmp.join("project");
    let external = tmp.join("external");
    std::fs::create_dir_all(project.join(".agents/skills")).unwrap();
    std::fs::create_dir_all(&external).unwrap();
    let regular_file = tmp.join("not-a-root");
    std::fs::write(&regular_file, "not a directory").unwrap();

    let roots = super::registry::valid_extra_skill_roots(
        Some(&project),
        &[
            external.clone(),
            external.join("."),
            project.join(".agents/skills"),
            project.join(".agents"),
            tmp.join("missing"),
            regular_file,
        ],
    );
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].config_index, 0);
    assert_eq!(roots[0].path, std::fs::canonicalize(&external).unwrap());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn companion_reads_require_inventory_identity_generation_and_containment() {
    let tmp = std::env::temp_dir().join(format!("koma-skill-companion-{}", uuid::Uuid::new_v4()));
    let project = tmp.join("project");
    let skill_dir = project.join(".agents/skills/demo");
    std::fs::create_dir_all(skill_dir.join("refs")).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\ndescription: Demo\n---\nBody",
    )
    .unwrap();
    std::fs::write(skill_dir.join("refs/guide.md"), "Guide").unwrap();
    std::fs::write(tmp.join("secret.md"), "secret").unwrap();

    let registry = SkillRegistry::load_isolated(Some(&project), &[]);
    let skill = registry.get("demo").unwrap();
    assert_eq!(skill.companion_files, vec!["refs/guide.md"]);
    assert_eq!(
        registry
            .read_companion_text(&skill.skill_id, &skill.generation, "refs/guide.md")
            .unwrap(),
        "Guide"
    );
    assert!(registry
        .read_companion_text(&skill.skill_id, &skill.generation, "../secret.md")
        .is_err());
    assert!(registry
        .read_companion_text(&skill.skill_id, "stale", "refs/guide.md")
        .is_err());
    assert!(registry
        .read_companion_text("wrong", &skill.generation, "refs/guide.md")
        .is_err());

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── Search tests ───────────────────────────────────────────────────────────

fn def(name: &str, description: &str, extra: &str) -> super::def::SkillDef {
    let content = format!("---\ndescription: {description}\n{extra}---\nbody of {name}");
    parse_skill(name, &content, SkillSource::Global, format!("/tmp/{name}.md").into(), None).unwrap()
}

fn big_registry() -> SkillRegistry {
    let mut defs: Vec<_> = (0..1000).map(|i| def(&format!("filler-{i}"), &format!("Generic helper number {i} for odd chores."), "")).collect();
    defs.push(def("jwt-review", "Review how a service signs and checks JSON web tokens.", "tags: [auth, token]\n"));
    defs.push(def("sql-injection-hunt", "Look for unsafe string-built database queries.", ""));
    SkillRegistry::from_defs(defs)
}

#[test]
fn search_finds_the_right_skill_among_a_thousand() {
    use super::search::Scope;
    let reg = big_registry();
    let (hits, total) = reg.search("check the JWT token handling", &Scope::default(), 0, 5);
    assert_eq!(hits[0].name, "jwt-review");
    assert!(total >= 1);
    let (hits, _) = reg.search("database queries injection", &Scope::default(), 0, 5);
    assert_eq!(hits[0].name, "sql-injection-hunt");
}

#[test]
fn search_by_tag_and_exact_name() {
    use super::search::Scope;
    let reg = big_registry();
    assert_eq!(reg.search("auth", &Scope::default(), 0, 3).0[0].name, "jwt-review");
    assert_eq!(reg.search("jwt-review", &Scope::default(), 0, 3).0[0].name, "jwt-review");
}

#[test]
fn an_empty_query_lists_nothing() {
    use super::search::Scope;
    let reg = big_registry();
    assert_eq!(reg.search("", &Scope::default(), 0, 5), (vec![], 0));
    assert_eq!(reg.search("the and", &Scope::default(), 0, 5).1, 0);
}

#[test]
fn search_pages_and_counts() {
    use super::search::Scope;
    let reg = big_registry();
    let (first, total) = reg.search("helper chores", &Scope::default(), 0, 10);
    let (second, _) = reg.search("helper chores", &Scope::default(), 10, 10);
    assert_eq!(first.len(), 10);
    assert!(total > 900);
    assert!(first.iter().all(|a| second.iter().all(|b| a.name != b.name)));
}

#[test]
fn scope_hides_what_a_role_may_not_see() {
    use super::search::Scope;
    let reg = big_registry();
    let only_sql = Scope::new(&["sql-*".to_string()]);
    assert!(only_sql.allows("sql-injection-hunt"));
    assert!(!only_sql.allows("jwt-review"));
    assert!(reg.search("jwt token", &only_sql, 0, 5).0.is_empty());
    assert_eq!(reg.search("database queries", &only_sql, 0, 5).0[0].name, "sql-injection-hunt");
    assert!(Scope::new(&["*-review".to_string()]).allows("jwt-review"));
    assert!(Scope::new(&["a*c".to_string()]).allows("abc"));
    assert!(!Scope::new(&["a*c".to_string()]).allows("abd"));
}

#[test]
fn pinned_skills_are_listed_for_the_prompt() {
    use super::search::Scope;
    let reg = SkillRegistry::from_defs(vec![def("daily", "Daily routine.", "pinned: true\n"), def("rare", "Rare thing.", "")]);
    let pinned = reg.pinned(&Scope::default(), 8);
    assert_eq!(pinned.len(), 1);
    assert_eq!(pinned[0].name, "daily");
}

#[test]
fn a_clear_winner_needs_no_rerank() {
    use super::search::{decisive, Hit};
    let h = |n: &str, s| Hit { name: n.into(), score: s };
    assert!(decisive(&[]));
    assert!(decisive(&[h("a", 5)]));
    assert!(decisive(&[h("a", 40), h("b", 10)]));
    assert!(!decisive(&[h("a", 14), h("b", 12)]));
}

#[test]
fn the_prompt_does_not_grow_with_the_number_of_skills() {
    use super::search::Scope;
    let few = SkillRegistry::from_defs((0..3).map(|i| def(&format!("s{i}"), "A skill.", "")).collect());
    let many = big_registry();
    assert_eq!(few.pinned(&Scope::default(), 8).len(), 0);
    let prompt = |reg: &SkillRegistry| crate::resources::build_system_prompt(None, None, None, reg.prompt_text().as_deref());
    let (a, b) = (prompt(&few), prompt(&many));
    assert_eq!(a, b, "a thousand skills must read the same as three");
    assert!(b.contains("skill_find") && !b.contains("filler-5"));
    // No skills at all: no skills section.
    assert!(!prompt(&SkillRegistry::from_defs(vec![])).contains("\n\n# Skills\n"));
    // Pinned ones are shown, and only those.
    let pinned = SkillRegistry::from_defs(vec![def("daily", "Daily routine.", "pinned: true\n"), def("rare", "Rare.", "")]);
    let p = prompt(&pinned);
    assert!(p.contains("- daily: Daily routine.") && !p.contains("- rare:"));
}
