use super::{Operation, Request};
use crate::app::runtime::client::file_ops;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn root(request: &Request) -> Result<PathBuf> {
    let path = Path::new(&request.workspace.root);
    anyhow::ensure!(path.is_absolute(), "Workspace root must be absolute");
    let root = path.canonicalize().context("Workspace is unavailable")?;
    anyhow::ensure!(root.is_dir(), "Workspace root is not a directory");
    Ok(root)
}

pub(super) fn execute(request: &Request) -> Result<Value> {
    // Existing processes remain inspectable/stoppable even after root deletion.
    match &request.operation {
        Operation::LspReleaseClient => {
            return super::language::release(&request.workspace).map_err(anyhow::Error::msg)
        }
        Operation::TestRuns => return super::tests::runs(&request.workspace),
        Operation::TestResults { run_id } => {
            return super::tests::results(&request.workspace, run_id)
        }
        Operation::TestStop { run_id } => return super::tests::stop(&request.workspace, run_id),
        Operation::DebugSessions => return super::debug::sessions(&request.workspace),
        Operation::DebugEvents { session_id, after } => {
            return super::debug::events(&request.workspace, session_id, *after)
        }
        Operation::DebugRequest {
            session_id,
            command,
            arguments,
            generation,
        } => {
            return super::debug::request(
                &request.workspace,
                session_id,
                command,
                arguments,
                *generation,
            )
        }
        Operation::DebugStop { session_id } => {
            return super::debug::stop(&request.workspace, session_id)
        }
        Operation::TaskInput { run_id, data } => {
            return super::tasks::input(&request.workspace, run_id, data)
        }
        Operation::TaskResize { run_id, rows, cols } => {
            return super::tasks::resize(&request.workspace, run_id, *rows, *cols)
        }
        Operation::TaskRuns => return super::tasks::runs(&request.workspace),
        Operation::TaskStop { run_id } => return super::tasks::stop(&request.workspace, run_id),
        Operation::TaskOutput { run_id, after } => {
            return super::tasks::output(&request.workspace, run_id, *after)
        }
        _ => {}
    }
    let canonical = root(request)?;
    let workdirs = vec![PathBuf::from(&request.workspace.root)];
    let r = &request.workspace.root;
    match &request.operation {
        Operation::LspReleaseClient => unreachable!(),
        Operation::ServiceInfo | Operation::ServiceUpgrade => unreachable!(),
        Operation::Hello => Ok(json!({"protocol":1,"root":canonical.to_string_lossy(),
            "capabilities":["paths","read","save","config","tasks","packs","debug"], "version":env!("CARGO_PKG_VERSION")})),
        Operation::TaskDefinitions => super::tasks::definitions(&canonical),
        Operation::TaskStart {
            task_id,
            fingerprint,
        } => super::tasks::start(&request.workspace, &canonical, task_id, fingerprint),
        Operation::Read { path } => Ok(serde_json::to_value(file_ops::exec_file_read(
            r,
            path,
            &request.id,
            &workdirs,
        ))?),
        Operation::File { body } => file_operation(request, body, &workdirs),
        Operation::Watch => super::watch::watch(&request.workspace, &canonical),
        Operation::ResourceJournals => super::resources::journals(&canonical),
        Operation::ResourceRecoveryPreview { transaction_id } => {
            super::resources::recovery_preview(&canonical, transaction_id)
        }
        Operation::ResourceRecover {
            transaction_id,
            expected,
        } => super::resources::recover(&canonical, transaction_id, expected),
        Operation::ResourceUndo { transaction_id } => {
            super::resources::undo(&canonical, transaction_id)
        }
        Operation::ResourceInspect { paths } => super::resources::inspect(&canonical, paths),
        Operation::ResourceApply { changes } => super::resources::apply(
            &canonical,
            &serde_json::from_value::<Vec<super::resources::Change>>(changes.clone())?,
        ),
        Operation::TestDefinitions => super::tests::definitions(&canonical),
        Operation::TestStart {
            profile_id,
            fingerprint,
            mode,
            previous,
            selection,
        } => super::tests::start(
            &request.workspace,
            &canonical,
            profile_id,
            fingerprint,
            mode,
            previous.as_deref(),
            selection,
        ),
        Operation::TestDebug {
            run_id,
            item_id,
            breakpoints,
        } => super::tests::debug(&request.workspace, &canonical, run_id, item_id, breakpoints),
        Operation::TestRuns | Operation::TestResults { .. } | Operation::TestStop { .. } => {
            unreachable!()
        }
        Operation::DebugDefinitions => super::debug::definitions(&canonical),
        Operation::DebugStart {
            profile_id,
            fingerprint,
            file,
            breakpoints,
        } => super::debug::start(
            &request.workspace,
            &canonical,
            profile_id,
            fingerprint,
            file.as_deref(),
            breakpoints,
        ),
        Operation::DebugSessions
        | Operation::DebugEvents { .. }
        | Operation::DebugRequest { .. }
        | Operation::DebugStop { .. } => unreachable!(),
        Operation::Packs => super::packs::status(&canonical),
        Operation::PackPlan {
            pack_id,
            runtime,
            server,
        } => super::packs::plan(
            &request.workspace,
            &canonical,
            pack_id,
            *runtime,
            server.as_deref(),
        ),
        Operation::PackApply { plan_id } => {
            super::packs::apply(&request.workspace, &canonical, plan_id)
        }
        Operation::LspRestartWorkspace => {
            super::language::restart(&request.workspace).map_err(anyhow::Error::msg)
        }
        Operation::EnvironmentSelect {
            language,
            executable,
            fingerprint,
            server,
        } => {
            let config = super::environment::select(
                &canonical,
                language,
                executable,
                fingerprint,
                server.as_deref(),
            )?;
            super::language::restart(&request.workspace).map_err(anyhow::Error::msg)?;
            Ok(config)
        }
        Operation::Save {
            path,
            content,
            fingerprint,
        } => Ok(serde_json::to_value(file_ops::exec_file_save(
            r,
            path,
            content,
            fingerprint,
            &request.id,
            &workdirs,
        ))?),
        Operation::Paths { query } => paths(&canonical, query),
        Operation::Inspect { paths } => {
            anyhow::ensure!(
                paths.len() <= 32,
                "Inspect at most 32 documents per request"
            );
            Ok(json!(paths.iter().map(|path| {
                let read = file_ops::exec_file_read(r, path, &request.id, &workdirs);
                json!({"path":path,"fingerprint":read.fingerprint,"error":read.error,"binary":read.binary,"tooLarge":read.too_large})
            }).collect::<Vec<_>>()))
        }
        Operation::ReplacePreview { options } => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct Options {
                query: String,
                replacement: String,
                case_sensitive: bool,
                whole_word: bool,
                is_regex: bool,
                include_glob: Option<String>,
                exclude_glob: Option<String>,
            }
            let options: Options = serde_json::from_value(options.clone())?;
            use crate::app::runtime::client::content_search::{preview_replacements, ContentQuery};
            preview_replacements(
                ContentQuery {
                    root: r,
                    path: "",
                    query: &options.query,
                    case_sensitive: options.case_sensitive,
                    whole_word: options.whole_word,
                    is_regex: options.is_regex,
                    include_glob: options.include_glob.as_deref(),
                    exclude_glob: options.exclude_glob.as_deref(),
                    request_id: &request.id,
                },
                &options.replacement,
                &workdirs,
            )
            .map_err(anyhow::Error::msg)
        }
        Operation::ConfigEnsure => {
            let dir = canonical.join(".koma");
            std::fs::create_dir_all(&dir)?;
            anyhow::ensure!(
                dir.canonicalize()?.starts_with(&canonical),
                "Coding directory is outside the workspace"
            );
            let path = dir.join("coding.json");
            if path.exists() {
                anyhow::ensure!(
                    path.canonicalize()?.starts_with(&canonical),
                    "Coding configuration is outside the workspace"
                );
            } else {
                use std::io::Write;
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                {
                    Ok(mut file) => {
                        file.write_all(b"{\n  \"version\": 1,\n  \"editor\": {},\n  \"snippets\": {},\n  \"keybindings\": []\n}\n")?;
                        file.sync_all()?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Ok(Value::Null)
        }
        Operation::ConfigRead => read_config(&canonical),
        Operation::ConfigWrite { config } => {
            validate_config(config)?;
            let dir = canonical.join(".koma");
            std::fs::create_dir_all(&dir)?;
            anyhow::ensure!(
                dir.canonicalize()?.starts_with(&canonical),
                "Coding directory is outside the workspace"
            );
            let dest = dir.join("coding.json");
            if dest.exists() {
                anyhow::ensure!(
                    dest.canonicalize()?.starts_with(&canonical),
                    "Coding configuration is outside the workspace"
                );
            }
            let bytes = serde_json::to_vec_pretty(config)?;
            anyhow::ensure!(
                bytes.len() <= 1024 * 1024,
                "Coding configuration exceeds 1 MiB"
            );
            crate::coding::persistence::atomic_write(&dest, &bytes)?;
            Ok(config.clone())
        }
        Operation::LspEditPreview { ticket } => {
            super::language::edit_preview(&request.workspace, ticket).map_err(anyhow::Error::msg)
        }
        Operation::LspEditReply {
            ticket,
            applied,
            reason,
        } => crate::lsp::client::reply_workspace_edit(
            &request.workspace.root,
            ticket,
            *applied,
            reason.as_deref(),
        )
        .map(|_| json!({}))
        .map_err(anyhow::Error::msg),
        Operation::LspCommand { path, params } => {
            super::language::command(&request.workspace, path, params).map_err(anyhow::Error::msg)
        }
        Operation::LspQuery {
            path,
            method,
            params,
        } => super::language::query(&request.workspace, path, method, params)
            .map_err(anyhow::Error::msg),
        Operation::Lsp { body } => {
            super::language::dispatch(&request.workspace, body).map_err(anyhow::Error::msg)
        }
        _ => anyhow::bail!("Operation is not a workspace operation"),
    }
}

pub(super) fn read_config(root: &Path) -> Result<Value> {
    let path = root.join(".koma/coding.json");
    if !path.exists() {
        return Ok(json!({"version":1}));
    }
    let actual = path.canonicalize()?;
    anyhow::ensure!(
        actual.starts_with(root),
        "Coding configuration is outside the workspace"
    );
    anyhow::ensure!(
        actual.metadata()?.len() <= 1024 * 1024,
        "Coding configuration exceeds 1 MiB"
    );
    let config: Value = serde_json::from_slice(&std::fs::read(actual)?)?;
    validate_config(&config)?;
    Ok(config)
}

fn validate_config(config: &Value) -> Result<()> {
    anyhow::ensure!(config.is_object(), "Coding configuration must be an object");
    anyhow::ensure!(
        config.get("version").and_then(Value::as_u64) == Some(1),
        "Unsupported coding configuration version"
    );
    for field in [
        "editor",
        "languages",
        "toolchains",
        "environment",
        "snippets",
    ] {
        if let Some(v) = config.get(field) {
            anyhow::ensure!(v.is_object(), "{field} must be an object");
        }
    }
    for field in ["tasks", "debug", "tests", "keybindings"] {
        if let Some(v) = config.get(field) {
            anyhow::ensure!(v.is_array(), "{field} must be an array");
        }
    }
    Ok(())
}

fn paths(root: &Path, query: &str) -> Result<Value> {
    let needle = query.to_lowercase();
    let mut matches = Vec::new();
    let mut scanned = 0;
    let mut truncated = false;
    let mut walk = ignore::WalkBuilder::new(root);
    walk.hidden(false).follow_links(false).max_depth(Some(64));
    walk.filter_entry(|e| {
        e.file_type().is_none_or(|f| !f.is_dir())
            || !matches!(
                e.file_name().to_str(),
                Some(".git" | ".koma" | "node_modules" | "target" | ".venv")
            )
    });
    for entry in walk.build().flatten() {
        if !entry.file_type().is_some_and(|f| f.is_file()) {
            continue;
        }
        scanned += 1;
        if scanned > 100_000 {
            truncated = true;
            break;
        }
        let path = entry
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if let Some(score) = fuzzy_score(&path.to_lowercase(), &needle) {
            matches.push((score, path));
        }
    }
    matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    truncated |= matches.len() > 200;
    matches.truncate(200);
    Ok(
        json!({"paths":matches.into_iter().map(|(_, path)| path).collect::<Vec<_>>(),"truncated":truncated}),
    )
}

fn fuzzy_score(path: &str, query: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(-(path.len() as i64));
    }
    let mut offset = 0;
    let mut score = 0;
    for c in query.chars() {
        let gap = path.get(offset..)?.find(c)?;
        score += if gap == 0 { 12 } else { -(gap as i64) };
        offset += gap + c.len_utf8();
    }
    if path.rsplit('/').next().unwrap_or(path).starts_with(query) {
        score += 100;
    }
    Some(score - path.len() as i64 / 4)
}

fn file_operation(request: &Request, body: &Value, workdirs: &[PathBuf]) -> Result<Value> {
    #[derive(serde::Deserialize)]
    #[serde(tag = "r", rename_all_fields = "camelCase")]
    enum FileOperation {
        #[serde(rename = "FileDownloadBytes")]
        DownloadBytes { path: String },
        #[serde(rename = "FileTree")]
        Tree { path: String },
        #[serde(rename = "FileRead")]
        Read { path: String },
        #[serde(rename = "FileSave")]
        Save {
            path: String,
            content: String,
            expected_fingerprint: String,
        },
        #[serde(rename = "FileCreate")]
        Create { path: String, kind: String },
        #[serde(rename = "FileRename")]
        Rename { old_path: String, new_path: String },
        #[serde(rename = "FileDelete")]
        Delete { path: String },
        #[serde(rename = "FileWriteBytes")]
        WriteBytes {
            path: String,
            bytes_b64: String,
            overwrite: bool,
        },
        #[serde(rename = "FileContentSearch")]
        ContentSearch {
            path: String,
            query: String,
            case_sensitive: bool,
            whole_word: bool,
            is_regex: bool,
            include_glob: Option<String>,
            exclude_glob: Option<String>,
        },
    }
    let r = &request.workspace.root;
    let id = &request.id;
    Ok(
        match serde_json::from_value::<FileOperation>(body.clone())? {
            FileOperation::DownloadBytes { path } => {
                serde_json::to_value(file_ops::exec_file_download_bytes(r, &path, id, workdirs))?
            }
            FileOperation::Tree { path } => {
                serde_json::to_value(file_ops::exec_file_tree(r, &path, id, workdirs))?
            }
            FileOperation::Read { path } => {
                serde_json::to_value(file_ops::exec_file_read(r, &path, id, workdirs))?
            }
            FileOperation::Save {
                path,
                content,
                expected_fingerprint,
            } => serde_json::to_value(file_ops::exec_file_save(
                r,
                &path,
                &content,
                &expected_fingerprint,
                id,
                workdirs,
            ))?,
            FileOperation::Create { path, kind } => {
                serde_json::to_value(file_ops::exec_file_create(r, &path, &kind, id, workdirs))?
            }
            FileOperation::Rename { old_path, new_path } => serde_json::to_value(
                file_ops::exec_file_rename(r, &old_path, &new_path, id, workdirs),
            )?,
            FileOperation::Delete { path } => {
                serde_json::to_value(file_ops::exec_file_delete(r, &path, id, workdirs))?
            }
            FileOperation::WriteBytes {
                path,
                bytes_b64,
                overwrite,
            } => serde_json::to_value(file_ops::exec_file_write_bytes(
                r, &path, &bytes_b64, overwrite, id, workdirs,
            ))?,
            FileOperation::ContentSearch {
                path,
                query,
                case_sensitive,
                whole_word,
                is_regex,
                include_glob,
                exclude_glob,
            } => {
                use crate::app::runtime::client::content_search::{
                    exec_file_content_search, ContentQuery,
                };
                serde_json::to_value(exec_file_content_search(
                    ContentQuery {
                        root: r,
                        path: &path,
                        query: &query,
                        case_sensitive,
                        whole_word,
                        is_regex,
                        include_glob: include_glob.as_deref(),
                        exclude_glob: exclude_glob.as_deref(),
                        request_id: id,
                    },
                    workdirs,
                ))?
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fuzzy_matches_unicode_and_prefers_basename() {
        assert!(fuzzy_score("src/école.ts", "éct").is_some());
        assert!(fuzzy_score("src/main.rs", "mx").is_none());
        assert!(fuzzy_score("src/main.rs", "main") > fuzzy_score("main/other.rs", "main"));
    }
    #[test]
    fn rejects_unknown_config_versions() {
        assert!(validate_config(&json!({"version":2})).is_err());
        assert!(validate_config(&json!({"version":1,"tasks":{}})).is_err());
    }
}
