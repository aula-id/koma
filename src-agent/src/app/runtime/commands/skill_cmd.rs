//! `/skill` command handler: activate/deactivate skills + open the hub.

use anyhow::Result;

use crate::app::mode::{Mode, SkillCmdState};
use crate::app::state::AppState;

/// Handle the `/skill` slash command: open the skill hub overlay.
///
/// Pre-seeds the hub's search query with any trailing argument.
pub(super) fn handle_skill(args: String, state: &mut AppState) -> Result<()> {
    let Some(session) = state.rest.fg().session.as_ref() else {
        state.rest.fg_mut().status = "no active session".into();
        return Ok(());
    };
    let registry = session.skills.clone();
    if registry.is_empty() {
        state.rest.fg_mut().status = "no skills found".into();
        return Ok(());
    }
    let active: std::collections::BTreeSet<String> =
        state.rest.fg().active_skills.keys().cloned().collect();
    let mut st = SkillCmdState::new(&registry, &active);
    // Pre-seed query from args
    if !args.trim().is_empty() {
        st.query = args.trim().to_string();
        st.refilter();
    }
    *state.mode_mut() = Mode::Skill(Box::new(st));
    Ok(())
}

/// Activate (load) a skill by name from the registry into active_skills.
/// Returns a status message. Errors are returned as Err.
pub(crate) fn activate_skill(
    state: &mut AppState,
    sess_idx: usize,
    name: &str,
) -> Result<String> {
    let skill_def = state.rest.sessions[sess_idx]
        .session
        .as_ref()
        .and_then(|sess| sess.skills.get(name))
        .ok_or_else(|| anyhow::anyhow!("skill '{}' not found in registry", name))?
        .clone();

    // Check if already active
    if state.rest.sessions[sess_idx]
        .active_skills
        .contains_key(&skill_def.name)
    {
        return Ok(format!("skill '{}' is already loaded", skill_def.name));
    }

    let body = skill_def.body.clone();
    let skill_dir = skill_def.skill_dir.clone();

    // Build companion inventory when dir-form (same logic as intercept_skill in guard.rs)
    let companion_msg = skill_dir.as_ref().map(|dir| {
        list_companions(dir, &skill_def.name)
    });

    state.rest.sessions[sess_idx]
        .active_skills
        .insert(
            skill_def.name.clone(),
            crate::app::state::ActiveSkill { body, skill_dir },
        );

    Ok(match companion_msg {
        Some(msg) => msg,
        None => format!("loaded skill '{}' — body injected into context.", skill_def.name),
    })
}

/// Deactivate (unload) a skill by name from active_skills.
pub(crate) fn deactivate_skill(
    state: &mut AppState,
    sess_idx: usize,
    name: &str,
) -> String {
    state.rest.sessions[sess_idx].active_skills.remove(name);
    format!("unloaded skill '{name}'.")
}

/// List companion files in a skill directory — inlined from
/// `guard::list_companions` (which has limited visibility). Lists non-recursive
/// one level + one level of subdirs, excluding the entry file.
fn list_companions(skill_dir: &std::path::Path, skill_name: &str) -> String {
    use std::fs;
    const ENTRY_FILES: &[&str] = &["SKILL.md", "skill.md"];
    const MAX_ENTRIES: usize = 50;

    let mut companions: Vec<String> = Vec::new();

    if let Ok(read_dir) = fs::read_dir(skill_dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();
            if path.is_file() && !ENTRY_FILES.contains(&file_name.as_str()) {
                companions.push(file_name);
            } else if path.is_dir() {
                if let Ok(sub) = fs::read_dir(&path) {
                    for sub_entry in sub.flatten() {
                        if sub_entry.path().is_file() {
                            let sub_name = format!(
                                "{}/{}",
                                file_name,
                                sub_entry.file_name().to_string_lossy()
                            );
                            companions.push(sub_name);
                        }
                    }
                }
            }
        }
    }
    companions.sort();
    let truncated = companions.len() > MAX_ENTRIES;
    companions.truncate(MAX_ENTRIES);

    let dir_display = skill_dir.display();
    let mut msg = format!("loaded skill '{skill_name}' — {dir_display}/");
    if companions.is_empty() {
        msg.push_str(" (no companion files)");
    } else {
        msg.push_str(" + companions:\n");
        for c in &companions {
            msg.push_str(&format!("  {c}\n"));
        }
        if truncated {
            msg.push_str("  …\n");
        }
    }
    msg
}
