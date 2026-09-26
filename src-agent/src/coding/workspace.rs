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
    let canonical = root(request)?;
    let workdirs = vec![PathBuf::from(&request.workspace.root)];
    let r = &request.workspace.root;
    match &request.operation {
        Operation::Hello => Ok(json!({"protocol":1,"root":canonical.to_string_lossy(),
            "capabilities":["paths","read","save","config"], "version":env!("CARGO_PKG_VERSION")})),
        Operation::Read { path } => Ok(serde_json::to_value(file_ops::exec_file_read(
            r,
            path,
            &request.id,
            &workdirs,
        ))?),
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
        Operation::ConfigRead => {
            let path = canonical.join(".koma/coding.json");
            if !path.exists() {
                return Ok(json!({"version":1}));
            }
            let actual = path.canonicalize()?;
            anyhow::ensure!(
                actual.starts_with(&canonical),
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
    for field in ["tasks", "debug", "tests"] {
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
