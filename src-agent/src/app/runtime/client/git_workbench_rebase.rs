use super::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Step {
    pub oid: String,
    pub action: String,
    pub message: String,
}
fn clean(root: &Path) -> Result<(), String> {
    if !git(root, &["status", "--porcelain=v1", "-z"])?.is_empty() {
        return Err("Commit or stash all changes before interactive rebase".into());
    }
    for name in [
        "rebase-merge",
        "rebase-apply",
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
    ] {
        let path = text(root, &["rev-parse", "--git-path", name])?;
        if root.join(path.trim()).exists() {
            return Err("Finish or abort the current Git operation first".into());
        }
    }
    Ok(())
}
pub(super) fn plan(root: &Path, base: &str) -> Result<Value, String> {
    clean(root)?;
    let base = oid(root, base)?;
    let head = oid(root, "HEAD")?;
    let branch = text(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map_err(|_| "Check out a local branch first")?;
    git(root, &["merge-base", "--is-ancestor", &base, &head])?;
    let range = format!("{base}..{head}");
    if !git(root, &["rev-list", "--merges", &range])?.is_empty() {
        return Err(
            "Interactive rebase currently requires a linear range without merge commits".into(),
        );
    }
    let hashes = text(root, &["rev-list", "--reverse", &range])?;
    if hashes.lines().count() > 500 {
        return Err("Select a range of at most 500 commits".into());
    }
    let mut steps = Vec::new();
    for hash in hashes.lines() {
        steps.push(Step {
            oid: hash.into(),
            action: "pick".into(),
            message: text(root, &["log", "-1", "--format=%B", hash])?
                .trim_end()
                .into(),
        });
    }
    if steps.is_empty() {
        return Err("Select a base before HEAD".into());
    }
    Ok(json!({"base":base,"head":head,"branch":branch.trim(),"steps":steps}))
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
pub(super) fn start(root: &Path, base: &str, head: &str, steps: &[Step]) -> Result<Value, String> {
    same_head(root, head)?;
    let plan = plan(root, base)?;
    let expected: std::collections::HashSet<String> =
        serde_json::from_value::<Vec<Step>>(plan["steps"].clone())
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|s| s.oid)
            .collect();
    let actual: std::collections::HashSet<_> = steps.iter().map(|s| s.oid.clone()).collect();
    if actual != expected || steps.len() != expected.len() {
        return Err("Rebase plan no longer matches this branch".into());
    }
    let mut picked = false;
    for step in steps {
        if !matches!(step.action.as_str(), "pick" | "reword" | "squash" | "drop") {
            return Err("Invalid rebase action".into());
        }
        if step.action == "squash" && !picked {
            return Err("The first retained commit cannot be squashed".into());
        }
        if matches!(step.action.as_str(), "reword" | "squash") && step.message.trim().is_empty() {
            return Err("Reword and squash require a commit message".into());
        }
        if step.action != "drop" {
            picked = true
        }
    }
    let dir = PathBuf::from(text(root, &["rev-parse", "--absolute-git-dir"])?.trim())
        .join(format!("koma-rebase-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut todo = String::new();
    for (i, step) in steps.iter().enumerate() {
        let action = match step.action.as_str() {
            "reword" => "pick",
            "squash" => "fixup",
            s => s,
        };
        todo.push_str(&format!("{action} {}\n", step.oid));
        if matches!(step.action.as_str(), "reword" | "squash") {
            let msg = dir.join(format!("message-{i}"));
            std::fs::write(&msg, &step.message).map_err(|e| e.to_string())?;
            todo.push_str(&format!(
                "exec {} __koma_git_editor message {}\n",
                quote(&exe.to_string_lossy()),
                quote(&msg.to_string_lossy())
            ));
        }
    }
    let todo_path = dir.join("todo");
    std::fs::write(&todo_path, todo).map_err(|e| e.to_string())?;
    let backup = format!("refs/koma/recovery/{}", uuid::Uuid::new_v4());
    git(
        root,
        &[
            "update-ref",
            "--create-reflog",
            "-m",
            "Koma interactive rebase backup",
            &backup,
            head,
        ],
    )?;
    let branch = plan["branch"].as_str().ok_or("Missing branch")?;
    super::super::git_remote::begin_rebase(root, branch, head);
    let sequence = format!(
        "{} __koma_git_editor sequence {}",
        quote(&exe.to_string_lossy()),
        quote(&todo_path.to_string_lossy())
    );
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .args([
            "-c",
            "rebase.updateRefs=false",
            "-c",
            "rebase.autoSquash=false",
            "rebase",
            "-i",
            base,
        ])
        .env("GIT_SEQUENCE_EDITOR", sequence)
        .env("GIT_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    crate::tool::shell::no_console_window(&mut cmd);
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        super::super::git_remote::finish_rebase(root, branch, &oid(root, "HEAD")?);
        let _ = std::fs::remove_dir_all(dir);
        Ok(json!({"ok":true,"backup":backup}))
    } else {
        Err(format!(
            "{}\nRecovery ref: {backup}",
            super::super::git::git_failure(&out, "Rebase paused")
        ))
    }
}
/// Git invokes this hidden helper before the normal CLI starts. Message text is
/// never interpolated into shell commands; Git's native sequencer owns recovery.
pub(crate) fn editor_helper() -> Option<anyhow::Result<()>> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("__koma_git_editor") {
        return None;
    }
    Some((|| {
        let kind = args
            .get(2)
            .ok_or_else(|| anyhow::anyhow!("Missing editor operation"))?;
        let source = args
            .get(3)
            .ok_or_else(|| anyhow::anyhow!("Missing editor input"))?;
        match kind.as_str() {
            "sequence" => {
                let target = args
                    .get(4)
                    .ok_or_else(|| anyhow::anyhow!("Missing Git todo path"))?;
                std::fs::copy(source, target)?;
            }
            "message" => {
                let mut cmd = Command::new("git");
                cmd.args(["commit", "--amend", "--file", source])
                    .env("GIT_EDITOR", "true")
                    .stdin(Stdio::null());
                crate::tool::shell::no_console_window(&mut cmd);
                let result = cmd.status()?;
                anyhow::ensure!(result.success(), "Could not update rebase commit message");
            }
            _ => anyhow::bail!("Unknown editor operation"),
        }
        Ok(())
    })())
}
