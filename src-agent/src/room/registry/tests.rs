use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;

/// A temp directory removed on drop.
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("koma-room-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn overlaid(tmp: &Tmp) -> RegistryHandle {
    RegistryHandle::with_overlay(tmp.path().to_path_buf()).expect("embedded registry loads")
}

const RED_ROLE_NARROWED: &str = r#"
id = "red"
display = "Lab Red (narrow)"
shelf_allow = ["get", "list", "put_claim"]
[chat]
model = "fast-model"
[work]
provider = "other"
model = "strong-model"
"#;

#[test]
fn embedded_registry_is_clean_and_complete() {
    let handle = RegistryHandle::embedded().expect("embedded files must be valid");
    let reg = handle.current();
    for id in ["chief", "red", "reviewer", "reporter", "clerk"] {
        let role = reg.role(id).unwrap_or_else(|| panic!("missing role {id}"));
        assert!(!role.prompt.is_empty(), "{id} prompt");
        assert!(!role.tools.is_empty(), "{id} tools");
        assert!(role
            .shelf_allow
            .iter()
            .all(|t| SHELF_TOOLS.contains(&t.as_str())));
        assert!(role
            .tools
            .iter()
            .all(|t| KNOWN_HANDLERS.contains(&t.handler.as_str())));
    }
    assert_eq!(reg.version, 1);
    assert_eq!(handle.report(), LoadReport::default());
}

#[test]
fn clerk_can_only_reach_the_chief() {
    let reg = RegistryHandle::embedded().expect("load").current();
    let clerk = reg.role("clerk").expect("clerk");
    assert_eq!(clerk.tools.len(), 1);
    assert_eq!(clerk.tools[0].handler, "clerk_flag");
    assert!(!clerk
        .shelf_allow
        .iter()
        .any(|t| t == "put_claim" || t == "put_verdict"));
}

#[test]
fn prompts_render_and_report_missing_variables() {
    let reg = RegistryHandle::embedded().expect("load").current();
    let text = reg
        .render(
            "wake",
            &[("goal_key", "g1"), ("goal", "scan"), ("open_goals", "none")],
        )
        .expect("render");
    assert!(text.contains("g1") && text.contains("scan") && !text.contains("{{"));
    let err = reg.render("wake", &[("goal_key", "g1")]).unwrap_err();
    assert_eq!(
        err,
        RenderError::MissingVar {
            prompt: "wake".into(),
            var: "goal".into()
        }
    );
    assert!(matches!(
        reg.render("nope", &[]),
        Err(RenderError::UnknownPrompt(_))
    ));
}

#[test]
fn template_rejects_malformed_placeholders() {
    assert!(template::validate("hello {{name}}").is_ok());
    assert!(template::validate("hello {{name").is_err());
    assert!(template::validate("hello {{Bad Name}}").is_err());
    assert!(template::validate("hello {{}}").is_err());
}

#[test]
fn overlay_replaces_prompt_and_bumps_version() {
    let tmp = Tmp::new("prompt");
    tmp.write("roles/red/prompt.md", "Custom red prompt.");
    let handle = overlaid(&tmp);
    let reg = handle.current();
    assert_eq!(reg.role("red").expect("red").prompt, "Custom red prompt.");
    assert_eq!(reg.version, 2);
    assert!(handle.report().warnings.is_empty());
}

#[test]
fn overlay_can_narrow_shelf_tools_and_set_lane_models() {
    let tmp = Tmp::new("narrow");
    tmp.write("roles/red/role.toml", RED_ROLE_NARROWED);
    let reg = overlaid(&tmp).current();
    let red = reg.role("red").expect("red");
    assert_eq!(red.shelf_allow, ["get", "list", "put_claim"]);
    assert_eq!(red.chat.model.as_deref(), Some("fast-model"));
    assert_eq!(red.work.provider.as_deref(), Some("other"));
    assert_eq!(red.work.model.as_deref(), Some("strong-model"));
}

#[test]
fn overlay_cannot_widen_shelf_tools() {
    let tmp = Tmp::new("widen");
    tmp.write(
        "roles/clerk/role.toml",
        "id = \"clerk\"\ndisplay = \"Clerk\"\nshelf_allow = [\"get\", \"put_verdict\"]\n",
    );
    let handle = overlaid(&tmp);
    let embedded = RegistryHandle::embedded().expect("load").current();
    assert_eq!(handle.current().role("clerk"), embedded.role("clerk"));
    assert!(handle.report().warnings.iter().any(|w| w.contains("widen")));
}

#[test]
fn overlay_cannot_bind_a_tool_to_another_roles_handler() {
    let tmp = Tmp::new("handler");
    tmp.write(
        "roles/clerk/tools.toml",
        "[[tool]]\nname = \"finish\"\nhandler = \"group_finish\"\ndescription = \"x\"\n[tool.schema]\ntype = \"object\"\n",
    );
    let handle = overlaid(&tmp);
    assert_eq!(
        handle.current().role("clerk").expect("clerk").tools[0].handler,
        "clerk_flag"
    );
    assert!(handle
        .report()
        .warnings
        .iter()
        .any(|w| w.contains("not one of this role")));
}

#[test]
fn overlay_cannot_use_unknown_handlers_or_add_roles() {
    let tmp = Tmp::new("unknown");
    tmp.write(
        "roles/red/tools.toml",
        "[[tool]]\nname = \"shell\"\nhandler = \"run_shell\"\ndescription = \"x\"\n[tool.schema]\ntype = \"object\"\n",
    );
    tmp.write("roles/ghost/prompt.md", "I am new.");
    let handle = overlaid(&tmp);
    let reg = handle.current();
    assert!(reg.role("ghost").is_none());
    assert_eq!(
        reg.role("red").expect("red").tools[0].handler,
        "group_task_complete"
    );
    let warnings = handle.report().warnings;
    assert!(warnings.iter().any(|w| w.contains("unknown handler")));
    assert!(warnings.iter().any(|w| w.contains("cannot add roles")));
}

#[test]
fn specialists_and_the_chief_run_host_tools_and_the_clerk_does_not() {
    let reg = RegistryHandle::embedded().expect("load").current();
    let red = reg.role("red").expect("red");
    assert!(red.tools.iter().any(|t| t.handler == "bash"));
    assert!(red.tools.iter().any(|t| t.handler == "web_search"));
    assert!(red.tools.iter().any(|t| t.handler == "browser_tabs"));
    let reviewer = reg.role("reviewer").expect("reviewer");
    assert!(reviewer.tools.iter().any(|t| t.handler == "bash"));
    assert!(reviewer
        .tools
        .iter()
        .all(|t| t.handler != "web_search" && t.handler != "browser_tabs"));
    let reporter = reg.role("reporter").expect("reporter");
    assert!(reporter.tools.iter().any(|t| t.handler == "read"));
    assert!(reporter.tools.iter().all(|t| t.handler != "web_fetch"));
    let chief = reg.role("chief").expect("chief");
    for tool in ["bash", "read", "write", "edit", "grep", "web_search", "browser_tabs"] {
        assert!(chief.tools.iter().any(|t| t.handler == tool), "chief lacks {tool}");
    }
    let clerk = reg.role("clerk").expect("clerk");
    assert!(
        clerk.tools.iter().all(|t| !HOST_HANDLERS.contains(&t.handler.as_str())),
        "the clerk has a host tool"
    );
}

#[test]
fn overlay_can_narrow_host_tools_but_not_add_sec_or_another_roles_surface() {
    let narrowed = Tmp::new("narrow-host");
    narrowed.write(
        "roles/red/tools.toml",
        "[[tool]]\nname = \"group_task_complete\"\nhandler = \"group_task_complete\"\ndescription = \"done\"\n[tool.schema]\ntype = \"object\"\n\n[[tool]]\nname = \"bash\"\nhandler = \"bash\"\ndescription = \"shell\"\n[tool.schema]\ntype = \"object\"\n",
    );
    let red = overlaid(&narrowed).current();
    let tools = &red.role("red").expect("red").tools;
    assert!(tools.iter().any(|t| t.handler == "bash"));
    assert!(tools.iter().all(|t| t.handler != "web_search"));

    let widened = Tmp::new("widen-host");
    widened.write(
        "roles/reviewer/tools.toml",
        "[[tool]]\nname = \"web_search\"\nhandler = \"web_search\"\ndescription = \"web\"\n[tool.schema]\ntype = \"object\"\n",
    );
    let handle = overlaid(&widened);
    assert!(handle
        .current()
        .role("reviewer")
        .expect("reviewer")
        .tools
        .iter()
        .all(|t| t.handler != "web_search"));
    assert!(handle
        .report()
        .warnings
        .iter()
        .any(|w| w.contains("not one of this role")));

    let sec = Tmp::new("sec-host");
    sec.write(
        "roles/red/tools.toml",
        "[[tool]]\nname = \"sec_http\"\nhandler = \"sec_http\"\ndescription = \"no\"\n[tool.schema]\ntype = \"object\"\n",
    );
    let handle = overlaid(&sec);
    assert!(handle
        .current()
        .role("red")
        .expect("red")
        .tools
        .iter()
        .any(|t| t.handler == "bash"));
    assert!(handle.report().warnings.iter().any(|w| w.contains("sec_")));
}

#[test]
fn broken_overlay_file_keeps_the_embedded_version() {
    let tmp = Tmp::new("broken");
    tmp.write("roles/red/role.toml", "this is = = not toml");
    tmp.write("prompts/wake.md", "Wake {{unclosed");
    let handle = overlaid(&tmp);
    let embedded = RegistryHandle::embedded().expect("load").current();
    assert_eq!(handle.current().role("red"), embedded.role("red"));
    assert_eq!(
        handle.current().prompts.get("wake"),
        embedded.prompts.get("wake")
    );
    assert_eq!(handle.report().warnings.len(), 2);
}

#[test]
fn overlay_adds_prompts_and_skills() {
    let tmp = Tmp::new("skill");
    tmp.write("prompts/custom.md", "Custom for {{who}}.");
    tmp.write(
        "skills/recon/SKILL.md",
        "---\nname: recon\ndescription: Map the surface\n---\nStart with passive sources.\n",
    );
    tmp.write("skills/recon/notes.md", "companion file, not a skill");
    tmp.write("skills/bad/SKILL.md", "no frontmatter here");
    let handle = overlaid(&tmp);
    let reg = handle.current();
    assert_eq!(
        reg.render("custom", &[("who", "red")]).expect("render"),
        "Custom for red."
    );
    let skill = reg.skills.get("recon").expect("skill");
    assert_eq!(skill.description, "Map the surface");
    assert_eq!(skill.body, "Start with passive sources.");
    assert!(!reg.skills.contains_key("bad"));
    assert_eq!(handle.report().warnings.len(), 1);
}

#[test]
fn reload_bumps_version_only_when_content_changes() {
    let tmp = Tmp::new("version");
    let handle = overlaid(&tmp);
    assert_eq!(handle.current().version, 1);
    assert!(!handle.reload(), "no overlay files, nothing changed");
    assert_eq!(handle.current().version, 1);
    tmp.write("roles/red/prompt.md", "First.");
    assert!(handle.reload());
    assert_eq!(handle.current().version, 2);
    assert!(!handle.reload(), "same content again");
    assert_eq!(handle.current().version, 2);
    tmp.write("roles/red/prompt.md", "Second.");
    assert!(handle.reload());
    assert_eq!(handle.current().version, 3);
}

#[test]
fn removing_an_overlay_file_returns_to_the_embedded_version() {
    let tmp = Tmp::new("remove");
    tmp.write("roles/red/prompt.md", "Custom.");
    let handle = overlaid(&tmp);
    assert_eq!(handle.current().role("red").expect("red").prompt, "Custom.");
    std::fs::remove_file(tmp.path().join("roles/red/prompt.md")).expect("rm");
    assert!(handle.reload());
    let embedded = RegistryHandle::embedded().expect("load").current();
    assert_eq!(handle.current().role("red"), embedded.role("red"));
}

#[test]
fn a_running_turn_keeps_its_snapshot_across_a_reload() {
    let tmp = Tmp::new("snapshot");
    let handle = overlaid(&tmp);
    let turn = handle.current();
    let before = turn.role("red").expect("red").prompt.clone();
    tmp.write("roles/red/prompt.md", "Changed mid-turn.");
    assert!(handle.reload());
    assert_eq!(turn.role("red").expect("red").prompt, before);
    assert_eq!(
        handle.current().role("red").expect("red").prompt,
        "Changed mid-turn."
    );
}

#[test]
fn oversized_and_foreign_files_are_ignored() {
    let tmp = Tmp::new("limits");
    tmp.write("roles/red/prompt.md", &"x".repeat(300 * 1024));
    tmp.write("roles/red/script.sh", "echo hi");
    let handle = overlaid(&tmp);
    let embedded = RegistryHandle::embedded().expect("load").current();
    assert_eq!(handle.current().role("red"), embedded.role("red"));
    assert!(handle
        .report()
        .warnings
        .iter()
        .any(|w| w.contains("larger than")));
}

#[cfg(unix)]
#[test]
fn symlinks_in_the_overlay_are_not_followed() {
    let tmp = Tmp::new("symlink");
    let outside = Tmp::new("outside");
    outside.write("prompt.md", "Smuggled prompt.");
    std::fs::create_dir_all(tmp.path().join("roles/red")).expect("mkdir");
    std::os::unix::fs::symlink(
        outside.path().join("prompt.md"),
        tmp.path().join("roles/red/prompt.md"),
    )
    .expect("symlink");
    let handle = overlaid(&tmp);
    assert_ne!(
        handle.current().role("red").expect("red").prompt,
        "Smuggled prompt."
    );
    assert!(handle
        .report()
        .warnings
        .iter()
        .any(|w| w.contains("symlink")));
}

#[test]
fn watcher_reloads_after_an_edit() {
    let tmp = Tmp::new("watch");
    let handle = Arc::new(overlaid(&tmp));
    let (tx, rx) = mpsc::channel();
    let _guard = spawn_watcher(Arc::clone(&handle), move |outcome| {
        let _ = tx.send(outcome);
    })
    .expect("watcher starts");

    tmp.write("roles/red/prompt.md", "Hot reloaded.");
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut seen = None;
    while std::time::Instant::now() < deadline {
        if let Ok(outcome) = rx.recv_timeout(Duration::from_millis(500)) {
            if outcome.changed {
                seen = Some(outcome);
                break;
            }
        }
    }
    let outcome = seen.expect("watcher should report the edit");
    assert_eq!(
        outcome.registry.role("red").expect("red").prompt,
        "Hot reloaded."
    );
    assert_eq!(
        handle.current().role("red").expect("red").prompt,
        "Hot reloaded."
    );
}

#[test]
fn the_session_overlay_lives_under_the_session_dir() {
    assert_eq!(
        overlay_dir_for(Path::new("/s/abc")),
        Path::new("/s/abc/room")
    );
}

// ---- avatars ----

const RED_API: &str = r#"
[[avatar]]
id = "red-api"
role = "red"
display = "Lab Red (API)"
[avatar.work]
model = "strong-model"
"#;

#[test]
fn the_default_room_gives_every_role_an_avatar_and_red_has_three() {
    let reg = RegistryHandle::embedded().expect("load").current();
    for role in reg.roles.keys() {
        assert!(!reg.avatars_of(role).is_empty(), "the {role} role has no avatar");
    }
    let reds: Vec<&str> = reg.avatars_of("red").iter().map(|a| a.id.as_str()).collect();
    assert_eq!(reds, ["red", "red-api", "red-web"]);
    for id in ["researcher", "re", "builder"] {
        assert!(reg.avatar(id).is_some(), "avatar {id}");
    }
    assert_eq!((reg.alias_of("chief"), reg.alias_of("red"), reg.alias_of("reporter"), reg.alias_of("researcher")), ("CF".to_string(), "LR".to_string(), "RR".to_string(), "RS".to_string()));
    for id in ["chief", "red", "reviewer", "reporter", "clerk"] {
        let a = reg.avatar(id).unwrap_or_else(|| panic!("avatar {id}"));
        assert_eq!(a.role, id);
        assert_eq!(reg.role_of(id).map(|r| r.id.as_str()), Some(id));
    }
    assert_eq!(reg.display_of("red"), "Lab Red");
    assert_eq!(reg.display_of("nobody"), "nobody");
}

#[test]
fn an_overlay_can_add_a_second_avatar_for_a_specialist_role() {
    let tmp = Tmp::new("avatar-add");
    tmp.write("avatars.toml", RED_API);
    let handle = overlaid(&tmp);
    let reg = handle.current();
    assert!(
        handle.report().warnings.is_empty(),
        "{:?}",
        handle.report().warnings
    );
    let ids: Vec<&str> = reg
        .avatars_of("red")
        .iter()
        .map(|a| a.id.as_str())
        .collect();
    assert_eq!(ids, ["red", "red-api", "red-web"]);
    assert_eq!(reg.display_of("red-api"), "Lab Red (API)");
    // a new avatar gets the role's prompt, tools and shelf permissions, nothing more
    assert_eq!(reg.role_of("red-api"), reg.role("red"));
}

#[test]
fn avatar_models_override_the_role_field_by_field_per_lane() {
    let tmp = Tmp::new("avatar-models");
    tmp.write("roles/red/role.toml", RED_ROLE_NARROWED); // role: chat fast-model, work other/strong-model
    tmp.write(
        "avatars.toml",
        "[[avatar]]\nid = \"red-api\"\nrole = \"red\"\n[avatar.work]\nmodel = \"override-model\"\neffort = \"high\"\n",
    );
    let reg = overlaid(&tmp).current();
    let work = reg.lane_model("red-api", LaneKind::Work);
    assert_eq!(
        work.provider.as_deref(),
        Some("other"),
        "inherited from the role"
    );
    assert_eq!(
        work.model.as_deref(),
        Some("override-model"),
        "the avatar wins"
    );
    assert_eq!(work.effort.as_deref(), Some("high"));
    let chat = reg.lane_model("red-api", LaneKind::Chat);
    assert_eq!(
        chat.model.as_deref(),
        Some("fast-model"),
        "the chat lane keeps the role's model"
    );
    assert_eq!(
        reg.lane_model("red", LaneKind::Chat).model.as_deref(),
        Some("fast-model")
    );
    assert_eq!(
        reg.lane_model("ghost", LaneKind::Work),
        LaneModel::default()
    );
}

#[test]
fn an_overlay_cannot_add_avatars_for_missing_roles_or_change_a_roles_owner() {
    let tmp = Tmp::new("avatar-bad");
    tmp.write(
        "avatars.toml",
        "[[avatar]]\nid = \"ghost\"\nrole = \"phantom\"\n[[avatar]]\nid = \"red\"\nrole = \"reviewer\"\n[[avatar]]\nid = \"red-2\"\nrole = \"red\"\n",
    );
    let handle = overlaid(&tmp);
    let reg = handle.current();
    assert!(reg.avatar("ghost").is_none());
    assert_eq!(
        reg.avatar("red").map(|a| a.role.as_str()),
        Some("red"),
        "an existing avatar keeps its role"
    );
    assert!(
        reg.avatar("red-2").is_some(),
        "valid entries in the same file still load"
    );
    let warnings = handle.report().warnings.join("\n");
    assert!(
        warnings.contains("unknown role 'phantom'") && warnings.contains("cannot change its role"),
        "{warnings}"
    );
}

#[test]
fn chief_and_clerk_stay_single() {
    for (name, body) in [
        (
            "second-chief",
            "[[avatar]]\nid = \"chief-2\"\nrole = \"chief\"\n",
        ),
        (
            "second-clerk",
            "[[avatar]]\nid = \"clerk-2\"\nrole = \"clerk\"\n",
        ),
        (
            "reserved-id",
            "[[avatar]]\nid = \"clerk\"\nrole = \"red\"\n",
        ),
        (
            "duplicate",
            "[[avatar]]\nid = \"a\"\nrole = \"red\"\n[[avatar]]\nid = \"a\"\nrole = \"red\"\n",
        ),
    ] {
        let tmp = Tmp::new(name);
        tmp.write("avatars.toml", body);
        let handle = overlaid(&tmp);
        let embedded = RegistryHandle::embedded().expect("load").current();
        assert_eq!(
            handle.current().avatars,
            embedded.avatars,
            "{name}: the whole file is rejected"
        );
        assert_eq!(
            handle.report().warnings.len(),
            1,
            "{name}: {:?}",
            handle.report().warnings
        );
    }
}
