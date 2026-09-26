//! Repository-bound operations for the extended Git GUI, shared with remote-git.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

#[path = "git_workbench_diff.rs"]
mod diff;
#[path = "git_workbench_rebase.rs"]
mod rebase;
pub(crate) use rebase::editor_helper;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Request {
    pub root: String,
    pub request_id: String,
    pub action: Action,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum Action {
    Diff {
        path: String,
        staged: bool,
        commit: Option<String>,
        old_path: Option<String>,
    },
    StageLines {
        path: String,
        staged: bool,
        token: String,
        lines: Vec<usize>,
    },
    Conflict {
        path: String,
    },
    Resolve {
        path: String,
        token: String,
        content: Option<String>,
        choice: Option<String>,
        stage: bool,
    },
    CommitMessage,
    Amend {
        message: String,
        head: String,
    },
    BranchRename {
        name: String,
        new_name: String,
    },
    BranchDelete {
        name: String,
        force: bool,
        remote: Option<String>,
    },
    TagCreate {
        name: String,
        target: String,
        message: Option<String>,
    },
    TagDelete {
        name: String,
        remote: Option<String>,
    },
    TagPush {
        name: String,
        remote: String,
    },
    Stashes,
    StashCreate {
        message: String,
        untracked: bool,
    },
    StashInspect {
        oid: String,
    },
    StashAction {
        oid: String,
        operation: String,
    },
    Reflog {
        skip: usize,
    },
    Recover {
        oid: String,
        name: String,
    },
    Blame {
        path: String,
    },
    Remotes,
    RemoteAdd {
        name: String,
        url: String,
    },
    RemoteEdit {
        name: String,
        new_name: String,
        url: String,
        push_url: String,
    },
    RemoteRemove {
        name: String,
    },
    RebasePlan {
        base: String,
    },
    RebaseRun {
        base: String,
        head: String,
        steps: Vec<rebase::Step>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Reply {
    pub root: String,
    pub request_id: String,
    pub data: Value,
    pub error: Option<String>,
}
impl Reply {
    pub fn error(request: &Request, message: impl Into<String>) -> Self {
        Self {
            root: request.root.clone(),
            request_id: request.request_id.clone(),
            data: Value::Null,
            error: Some(message.into()),
        }
    }
}
pub(crate) fn emit(push: &dyn Fn(String), reply: Reply) {
    let mut value = serde_json::to_value(reply).unwrap_or(Value::Null);
    value["k"] = json!("GitWorkbench");
    push(value.to_string());
}
pub(crate) fn handle(request: Request, session: Option<&str>) -> Reply {
    let Some(root) = super::git::repo_root_for(session) else {
        return Reply::error(&request, "No active repository");
    };
    if root != Path::new(&request.root) {
        return Reply::error(&request, "Repository changed; reopen this Git view");
    }
    let result = super::git::with_git_transaction(|_| execute(&root, &request.action));
    match result {
        Ok(data) => Reply {
            root: request.root,
            request_id: request.request_id,
            data,
            error: None,
        },
        Err(error) => Reply::error(&request, error),
    }
}
pub(crate) fn spawn(
    push: impl Fn(String) + Send + 'static,
    request: Request,
    session: Option<String>,
) {
    std::thread::spawn(move || emit(&push, handle(request, session.as_deref())));
}
fn run(root: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Output, String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(ssh) = super::git_remote::ssh_command_for(root) {
        cmd.env("GIT_SSH_COMMAND", ssh);
    }
    crate::tool::shell::no_console_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    // Write on a separate thread so a verbose Git child cannot deadlock stdout.
    let writer = input.map(|bytes| {
        let bytes = bytes.to_vec();
        let stdin = child.stdin.take();
        std::thread::spawn(move || {
            stdin
                .ok_or_else(|| "Git stdin unavailable".to_string())?
                .write_all(&bytes)
                .map_err(|e| e.to_string())
        })
    });
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if let Some(writer) = writer {
        writer
            .join()
            .map_err(|_| "Git input writer failed".to_string())??;
    }
    Ok(output)
}
fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = run(root, args, None)?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(super::git::git_failure(&out, "Git operation failed"))
    }
}
fn text(root: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(git(root, args)?).map_err(|_| "Content is not UTF-8 text".to_string())
}
fn done(root: &Path, args: &[&str]) -> Result<Value, String> {
    git(root, args)?;
    Ok(json!({"ok":true}))
}
fn oid(root: &Path, name: &str) -> Result<String, String> {
    if name.is_empty() || name.starts_with('-') || name.contains(['\0', '\n']) {
        return Err("Invalid revision".into());
    }
    Ok(text(
        root,
        &["rev-parse", "--verify", &format!("{name}^{{commit}}")],
    )?
    .trim()
    .to_string())
}
fn same_head(root: &Path, expected: &str) -> Result<(), String> {
    if oid(root, "HEAD")? != expected {
        return Err("HEAD changed; refresh and review again".into());
    }
    Ok(())
}
fn name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.starts_with('-')
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("Invalid name".into());
    }
    Ok(())
}
fn safe_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    use std::path::Component;
    if path.is_empty()
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Invalid repository path".into());
    }
    // Refuse symlink ancestors; reading a tracked symlink itself is handled as binary.
    let mut p = root.to_path_buf();
    for part in Path::new(path).components() {
        p.push(part);
        if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Symlink files use whole-file Git operations".into());
        }
    }
    Ok(p)
}
fn fingerprint(parts: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    for part in parts {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part);
    }
    format!("{:x}", h.finalize())
}
fn stash_ref(root: &Path, expected: &str) -> Result<String, String> {
    let list = text(root, &["stash", "list", "--format=%H %gd"])?;
    list.lines()
        .find_map(|l| {
            l.split_once(' ')
                .filter(|(hash, _)| *hash == expected)
                .map(|(_, r)| r.to_string())
        })
        .ok_or_else(|| "Stash changed or was removed; refresh the list".into())
}
fn execute(root: &Path, action: &Action) -> Result<Value, String> {
    use Action::*;
    match action {
        Diff {
            path,
            staged,
            commit,
            old_path,
        } => diff::inspect(root, path, *staged, commit.as_deref(), old_path.as_deref()),
        StageLines {
            path,
            staged,
            token,
            lines,
        } => diff::stage(root, path, *staged, token, lines),
        Conflict { path } => diff::conflict(root, path),
        Resolve {
            path,
            token,
            content,
            choice,
            stage,
        } => diff::resolve(
            root,
            path,
            token,
            content.as_deref(),
            choice.as_deref(),
            *stage,
        ),
        CommitMessage => {
            Ok(json!({"head":oid(root,"HEAD")?,"message":text(root,&["log","-1","--format=%B"])?}))
        }
        Amend { message, head } => {
            same_head(root, head)?;
            if message.trim().is_empty() {
                return Err("Commit message is empty".into());
            }
            let backup = format!("refs/koma/recovery/{}", uuid::Uuid::new_v4());
            git(root, &["update-ref", &backup, head])?;
            let result = done(root, &["commit", "--amend", "-m", message]);
            super::git_remote::invalidate_rebase_proofs(root);
            result
        }
        BranchRename {
            name: old,
            new_name,
        } => {
            name(old)?;
            name(new_name)?;
            done(root, &["branch", "-m", old, new_name])
        }
        BranchDelete {
            name: branch,
            force,
            remote,
        } => {
            name(branch)?;
            if let Some(remote) = remote {
                name(remote)?;
                done(
                    root,
                    &["push", remote, "--delete", &format!("refs/heads/{branch}")],
                )
            } else {
                done(root, &["branch", if *force { "-D" } else { "-d" }, branch])
            }
        }
        TagCreate {
            name: tag,
            target,
            message,
        } => {
            name(tag)?;
            let target = oid(root, target)?;
            match message {
                Some(message) => done(root, &["tag", "-a", tag, "-m", message, &target]),
                None => done(root, &["tag", tag, &target]),
            }
        }
        TagDelete { name: tag, remote } => {
            name(tag)?;
            if let Some(remote) = remote {
                name(remote)?;
                done(
                    root,
                    &["push", remote, "--delete", &format!("refs/tags/{tag}")],
                )
            } else {
                done(root, &["tag", "-d", tag])
            }
        }
        TagPush { name: tag, remote } => {
            name(tag)?;
            name(remote)?;
            done(
                root,
                &["push", remote, &format!("refs/tags/{tag}:refs/tags/{tag}")],
            )
        }
        Stashes => Ok(
            json!({"entries":text(root,&["stash","list","--format=%H%x09%gd%x09%gs"] )?.lines().filter_map(|l| {let p:Vec<_>=l.splitn(3,'\t').collect();(p.len()==3).then(||json!({"oid":p[0],"ref":p[1],"message":p[2]}))}).collect::<Vec<_>>()}),
        ),
        StashCreate { message, untracked } => {
            let mut args = vec!["stash", "push", "-m", message];
            if *untracked {
                args.push("--include-untracked")
            }
            done(root, &args)
        }
        StashInspect { oid: hash } => {
            let r = stash_ref(root, hash)?;
            Ok(
                json!({"patch":text(root,&["stash","show","--include-untracked","--no-ext-diff","--no-color","-p",&r])?}),
            )
        }
        StashAction {
            oid: hash,
            operation,
        } => {
            if !matches!(operation.as_str(), "apply" | "pop" | "drop") {
                return Err("Invalid stash operation".into());
            }
            let r = stash_ref(root, hash)?;
            done(root, &["stash", operation, &r])
        }
        Reflog { skip } => {
            let rows = text(
                root,
                &[
                    "reflog",
                    "show",
                    "--all",
                    "--date=iso-strict",
                    "--format=%H%x09%gd%x09%gs",
                    "-n",
                    "101",
                    &format!("--skip={skip}"),
                ],
            )?;
            let entries: Vec<_> = rows
                .lines()
                .filter_map(|l| {
                    let p: Vec<_> = l.splitn(3, '\t').collect();
                    (p.len() == 3).then(|| json!({"oid":p[0],"selector":p[1],"message":p[2]}))
                })
                .collect();
            Ok(
                json!({"hasMore":entries.len()>100,"entries":entries.into_iter().take(100).collect::<Vec<_>>()}),
            )
        }
        Recover {
            oid: hash,
            name: branch,
        } => {
            name(branch)?;
            let hash = oid(root, hash)?;
            done(root, &["branch", branch, &hash])
        }
        Blame { path } => diff::blame(root, path),
        Remotes => {
            let mut entries = Vec::new();
            for r in text(root, &["remote"])?.lines() {
                entries.push(json!({"name":r,"url":text(root,&["remote","get-url",r])?.trim(),"pushUrl":text(root,&["remote","get-url","--push",r])?.trim()}));
            }
            Ok(json!({"entries":entries}))
        }
        RemoteAdd { name: remote, url } => {
            name(remote)?;
            if url.is_empty() || url.starts_with('-') {
                return Err("Invalid remote URL".into());
            }
            done(root, &["remote", "add", remote, url])
        }
        RemoteEdit {
            name: remote,
            new_name,
            url,
            push_url,
        } => {
            name(remote)?;
            name(new_name)?;
            if [url, push_url]
                .iter()
                .any(|s| s.is_empty() || s.starts_with('-'))
            {
                return Err("Both remote URLs are required".into());
            }
            git(root, &["remote", "set-url", remote, url])?;
            git(root, &["remote", "set-url", "--push", remote, push_url])?;
            if remote != new_name {
                git(root, &["remote", "rename", remote, new_name])?;
            }
            Ok(json!({"ok":true}))
        }
        RemoteRemove { name: remote } => {
            name(remote)?;
            done(root, &["remote", "remove", remote])
        }
        RebasePlan { base } => rebase::plan(root, base),
        RebaseRun { base, head, steps } => rebase::start(root, base, head, steps),
    }
}
