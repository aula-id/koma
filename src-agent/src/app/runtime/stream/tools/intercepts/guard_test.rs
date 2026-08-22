#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::list_companions;

#[test]
fn lists_companions_and_skips_entry_files() {
    let tmp =
        std::env::temp_dir().join(format!("koma-guard-test-companions-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("SKILL.md"), "# skill body").unwrap();
    std::fs::write(tmp.join("helper.md"), "helper content").unwrap();
    std::fs::write(tmp.join("notes.txt"), "notes").unwrap();

    let msg = list_companions(&tmp, "test-skill");
    assert!(msg.contains("loaded skill 'test-skill'"));
    assert!(msg.contains("skill_dir:"));
    assert!(msg.contains("- helper.md"));
    assert!(msg.contains("- notes.txt"));
    // SKILL.md must NOT appear as a companion.
    assert!(!msg.contains("- SKILL.md"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn lists_subdir_companions_one_level_deep() {
    let tmp =
        std::env::temp_dir().join(format!("koma-guard-test-subdir-{}", std::process::id()));
    let refs = tmp.join("references");
    std::fs::create_dir_all(&refs).unwrap();
    std::fs::write(tmp.join("SKILL.md"), "# skill").unwrap();
    std::fs::write(refs.join("api.md"), "api ref").unwrap();
    std::fs::write(refs.join("deep.md"), "deep").unwrap();

    let msg = list_companions(&tmp, "subdir-skill");
    assert!(msg.contains("- references/api.md"));
    assert!(msg.contains("- references/deep.md"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn flat_skill_dir_shows_no_companions() {
    let tmp =
        std::env::temp_dir().join(format!("koma-guard-test-empty-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("SKILL.md"), "# skill").unwrap();
    // No other files.

    let msg = list_companions(&tmp, "empty-skill");
    assert!(msg.contains("(no companion files in skill directory)"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn nonexistent_dir_shows_no_companions() {
    let fake = std::path::PathBuf::from(format!(
        "/tmp/koma-guard-test-nonexistent-{}",
        std::process::id()
    ));
    let msg = list_companions(&fake, "ghost");
    assert!(msg.contains("(no companion files in skill directory)"));
}
