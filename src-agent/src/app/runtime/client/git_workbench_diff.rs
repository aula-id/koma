use super::*;
use base64::Engine;
const TEXT_CAP: usize = 2 * 1024 * 1024;
const IMAGE_CAP: usize = 16 * 1024 * 1024;

fn blob(root: &Path, spec: &str) -> Result<Option<Vec<u8>>, String> {
    let out = run(root, &["cat-file", "-e", spec], None)?;
    if !out.status.success() {
        return Ok(None);
    }
    let size = text(root, &["cat-file", "-s", spec])?
        .trim()
        .parse::<usize>()
        .map_err(|_| "Invalid blob size")?;
    if size > IMAGE_CAP {
        return Err("File too large to preview".into());
    }
    git(root, &["show", spec]).map(Some)
}
fn worktree(root: &Path, path: &str) -> Result<Option<Vec<u8>>, String> {
    let path = safe_path(root, path)?;
    match std::fs::metadata(&path) {
        Ok(m) if m.len() > IMAGE_CAP as u64 => return Err("File too large to preview".into()),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
        _ => {}
    }
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
fn bytes(v: &Option<Vec<u8>>) -> &[u8] {
    v.as_deref().unwrap_or_default()
}
fn snapshot(a: &Option<Vec<u8>>, b: &Option<Vec<u8>>, index: &str) -> String {
    fingerprint(&[
        bytes(a),
        bytes(b),
        &[a.is_some() as u8, b.is_some() as u8],
        index.as_bytes(),
    ])
}
fn mime(path: &str) -> Option<&'static str> {
    match path.rsplit('.').next()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "ico" => Some("image/x-icon"),
        "avif" => Some("image/avif"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}
fn image_data(value: &Option<Vec<u8>>, mime: &str) -> Value {
    value.as_ref().map(|b|json!({"mime":mime,"size":b.len(),"base64":base64::engine::general_purpose::STANDARD.encode(b)})).unwrap_or(Value::Null)
}
fn utf8(value: &Option<Vec<u8>>) -> Result<String, String> {
    let b = bytes(value);
    if b.len() > TEXT_CAP {
        return Err("File too large for text diff".into());
    }
    if b.contains(&0) {
        return Err("Binary file — text operations unavailable".into());
    }
    String::from_utf8(b.to_vec()).map_err(|_| "Binary file — text operations unavailable".into())
}
fn sides(
    root: &Path,
    path: &str,
    staged: bool,
) -> Result<(Option<Vec<u8>>, Option<Vec<u8>>, String), String> {
    safe_path(root, path)?;
    let index = text(root, &["ls-files", "--stage", "-z", "--", path])?;
    if index
        .lines()
        .any(|l| l.split_whitespace().nth(2).is_some_and(|s| s != "0"))
    {
        return Err("Resolve this file's conflicts first".into());
    }
    let a = blob(root, &format!(":{path}"))?;
    let b = if staged {
        blob(root, &format!("HEAD:{path}"))?
    } else {
        worktree(root, path)?
    };
    Ok((a, b, index))
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    id: usize,
    kind: String,
    text: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Hunk {
    header: String,
    old_start: usize,
    rows: Vec<Row>,
}
fn hunks(root: &Path, a: &str, b: &str) -> Result<Vec<Hunk>, String> {
    let tmp = std::env::temp_dir().join(format!("koma-git-diff-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&tmp).map_err(|e| e.to_string())?;
    let result = (|| {
        std::fs::write(tmp.join("a"), a).map_err(|e| e.to_string())?;
        std::fs::write(tmp.join("b"), b).map_err(|e| e.to_string())?;
        let out = run(
            root,
            &[
                "diff",
                "--no-index",
                "--no-ext-diff",
                "--no-color",
                "--text",
                "--unified=3",
                &tmp.join("a").to_string_lossy(),
                &tmp.join("b").to_string_lossy(),
            ],
            None,
        )?;
        if !out.status.success() && out.status.code() != Some(1) {
            return Err(super::super::git::git_failure(
                &out,
                "Unable to compare text",
            ));
        }
        let patch = String::from_utf8(out.stdout).map_err(|e| e.to_string())?;
        parse_hunks(&patch)
    })();
    let _ = std::fs::remove_dir_all(tmp);
    result
}
fn parse_hunks(patch: &str) -> Result<Vec<Hunk>, String> {
    let mut result: Vec<Hunk> = Vec::new();
    let mut id = 0;
    for line in patch.split_inclusive('\n') {
        if line.starts_with("@@ ") {
            let start = line
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.trim_start_matches('-').split(',').next())
                .and_then(|s| s.parse::<usize>().ok())
                .ok_or("Invalid diff hunk")?;
            result.push(Hunk {
                header: line.trim_end().into(),
                old_start: if line
                    .split_whitespace()
                    .nth(1)
                    .is_some_and(|s| s.ends_with(",0"))
                {
                    start
                } else {
                    start.saturating_sub(1)
                },
                rows: Vec::new(),
            });
        } else if let Some(h) = result.last_mut() {
            if line.starts_with("\\ No newline") {
                if let Some(r) = h.rows.last_mut() {
                    r.text.pop();
                }
                continue;
            }
            let k = &line[..1];
            if matches!(k, " " | "+" | "-") {
                id += 1;
                h.rows.push(Row {
                    id,
                    kind: k.into(),
                    text: line[1..].into(),
                });
            }
        }
    }
    Ok(result)
}

fn selected_text(original: &str, hunks: &[Hunk], selection: &[usize]) -> Result<String, String> {
    let source: Vec<_> = original.split_inclusive('\n').collect();
    let mut out = String::new();
    let mut cursor = 0;
    let selected: std::collections::HashSet<_> = selection.iter().copied().collect();
    let valid: std::collections::HashSet<_> = hunks
        .iter()
        .flat_map(|h| h.rows.iter())
        .filter(|r| r.kind != " ")
        .map(|r| r.id)
        .collect();
    if selected.is_empty() || !selected.is_subset(&valid) {
        return Err("Select changed lines from the current diff".into());
    }
    for h in hunks {
        if h.old_start < cursor || h.old_start > source.len() {
            return Err("Diff position changed".into());
        }
        for l in &source[cursor..h.old_start] {
            out.push_str(l)
        }
        cursor = h.old_start;
        for r in &h.rows {
            match r.kind.as_str() {
                "+" => {
                    if selected.contains(&r.id) {
                        out.push_str(&r.text)
                    }
                }
                "-" => {
                    if !selected.contains(&r.id) {
                        out.push_str(&r.text)
                    }
                    cursor += 1;
                }
                _ => {
                    out.push_str(&r.text);
                    cursor += 1;
                }
            }
        }
    }
    for l in source.get(cursor..).ok_or("Diff exceeds source")? {
        out.push_str(l)
    }
    Ok(out)
}
// Partial operations are only meaningful for regular text with stable modes.
// Git clean filters and working-tree encodings can change unrelated bytes, so
// those files keep the existing whole-file staging controls.
fn partial_allowed(root: &Path, path: &str, staged: bool, index: &str) -> Result<bool, String> {
    if index.starts_with("160000") || index.starts_with("120000") {
        return Ok(false);
    }
    let attributes = git(
        root,
        &[
            "check-attr",
            "-z",
            "filter",
            "working-tree-encoding",
            "--",
            path,
        ],
    )?;
    let parts: Vec<_> = attributes.split(|b| *b == 0).collect();
    if parts
        .chunks_exact(3)
        .any(|p| p[2] != b"unspecified" && p[2] != b"unset")
    {
        return Ok(false);
    }
    let mut args = vec!["diff", "--raw", "-z", "--no-ext-diff", "--no-renames"];
    if staged {
        args.push("--cached");
    }
    args.extend(["--", path]);
    let raw = text(root, &args)?;
    for header in raw.split('\0').filter(|s| s.starts_with(':')) {
        let modes: Vec<_> = header.split_whitespace().take(2).collect();
        if modes.len() == 2
            && modes[0] != ":000000"
            && modes[1] != "000000"
            && &modes[0][1..] != modes[1]
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn target_text(
    root: &Path,
    path: &str,
    staged: bool,
    a: &Option<Vec<u8>>,
    b: &Option<Vec<u8>>,
) -> Result<String, String> {
    let raw = utf8(b)?;
    if staged || a.is_none() {
        return Ok(raw);
    }
    // Git supplies the canonical text after its CRLF/attributes conversion.
    // Applying all rows reconstructs that text without writing a temporary blob.
    let patch = text(
        root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--no-renames",
            "--text",
            "--unified=3",
            "--",
            path,
        ],
    )?;
    let rows = parse_hunks(&patch)?;
    let ids: Vec<_> = rows
        .iter()
        .flat_map(|h| h.rows.iter())
        .filter(|r| r.kind != " ")
        .map(|r| r.id)
        .collect();
    if ids.is_empty() {
        return utf8(a);
    }
    selected_text(&utf8(a)?, &rows, &ids)
}
pub(super) fn inspect(
    root: &Path,
    path: &str,
    staged: bool,
    commit: Option<&str>,
    old_path: Option<&str>,
) -> Result<Value, String> {
    safe_path(root, path)?;
    if let Some(p) = old_path {
        safe_path(root, p)?;
    }
    let (a, b, index) = if let Some(commit) = commit {
        let hash = oid(root, commit)?;
        (
            blob(root, &format!("{hash}^1:{}", old_path.unwrap_or(path)))?,
            blob(root, &format!("{hash}:{path}"))?,
            String::new(),
        )
    } else {
        let (a, mut b, index) = sides(root, path, staged)?;
        if staged {
            if let Some(old) = old_path {
                b = blob(root, &format!("HEAD:{old}"))?;
            }
        }
        (a, b, index)
    };
    let token = snapshot(&a, &b, &index);
    if let Some(mime) = mime(path) {
        if bytes(&a).len() + bytes(&b).len() > IMAGE_CAP {
            return Err("Image comparison exceeds 16 MiB".into());
        }
        return Ok(
            json!({"token":token,"image":true,"originalImage":image_data(&a,mime),"modifiedImage":image_data(&b,mime),"original": "","modified":"","hunks":[],"partial":false}),
        );
    }
    let original = utf8(&a)?;
    let partial =
        commit.is_none() && old_path.is_none() && partial_allowed(root, path, staged, &index)?;
    let modified = if partial {
        target_text(root, path, staged, &a, &b)?
    } else {
        utf8(&b)?
    };
    let rows = if partial {
        hunks(root, &original, &modified)?
    } else {
        Vec::new()
    };
    Ok(
        json!({"token":token,"original":original,"modified":modified,"hunks":rows,"image":false,"partial":partial}),
    )
}
pub(super) fn stage(
    root: &Path,
    path: &str,
    staged: bool,
    token: &str,
    lines: &[usize],
) -> Result<Value, String> {
    let (a, b, index) = sides(root, path, staged)?;
    if token != snapshot(&a, &b, &index) {
        return Err("File or index changed; refresh the diff before staging".into());
    }
    if !partial_allowed(root, path, staged, &index)? {
        return Err("Use whole-file staging for this file".into());
    }
    let source = utf8(&a)?;
    let target = target_text(root, path, staged, &a, &b)?;
    let hunks = hunks(root, &source, &target)?;
    let next = selected_text(&source, &hunks, lines)?;
    if b.is_none() && next.is_empty() {
        git(root, &["update-index", "--force-remove", "--", path])?;
    } else {
        let clean_path = format!("--path={path}");
        let mut args = vec!["hash-object", "-w", "--stdin"];
        // Untracked files have no Git diff to apply the clean conversion yet.
        if !staged && a.is_none() {
            args.push(&clean_path);
        }
        let result = run(root, &args, Some(next.as_bytes()))?;
        if !result.status.success() {
            return Err(super::super::git::git_failure(
                &result,
                "Cannot create staged blob",
            ));
        }
        let hash = String::from_utf8_lossy(&result.stdout).trim().to_string();
        let default_mode = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if std::fs::metadata(root.join(path))
                    .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
                {
                    "100755"
                } else {
                    "100644"
                }
            }
            #[cfg(not(unix))]
            {
                "100644"
            }
        };
        let head_entry = if staged && index.is_empty() {
            text(root, &["ls-tree", "-z", "HEAD", "--", path])?
        } else {
            String::new()
        };
        let mode = index
            .split_whitespace()
            .next()
            .or_else(|| head_entry.split_whitespace().next())
            .unwrap_or(default_mode);
        git(
            root,
            &["update-index", "--add", "--cacheinfo", mode, &hash, path],
        )?;
    }
    Ok(json!({"ok":true}))
}
pub(super) fn conflict(root: &Path, path: &str) -> Result<Value, String> {
    safe_path(root, path)?;
    let entries = text(root, &["ls-files", "--unmerged", "-z", "--", path])?;
    if entries.is_empty() {
        return Err("This file is no longer conflicted; refresh status".into());
    }
    if entries
        .split('\0')
        .any(|e| e.starts_with("120000") || e.starts_with("160000"))
    {
        return Err(
            "Resolve symlink and submodule conflicts with whole-file Git operations".into(),
        );
    }
    let base = blob(root, &format!(":1:{path}"))?;
    let current = blob(root, &format!(":2:{path}"))?;
    let incoming = blob(root, &format!(":3:{path}"))?;
    let working = worktree(root, path)?;
    let binary = [&base, &current, &incoming, &working]
        .iter()
        .any(|b| utf8(b).is_err());
    let token = snapshot(&current, &working, &entries);
    Ok(
        json!({"token":token,"base":utf8(&base).unwrap_or_default(),"current":utf8(&current).unwrap_or_default(),"incoming":utf8(&incoming).unwrap_or_default(),"result":utf8(&working).unwrap_or_default(),"currentExists":current.is_some(),"incomingExists":incoming.is_some(),"binary":binary}),
    )
}
pub(super) fn resolve(
    root: &Path,
    path: &str,
    token: &str,
    content: Option<&str>,
    choice: Option<&str>,
    stage: bool,
) -> Result<Value, String> {
    let before = conflict(root, path)?;
    if before["token"].as_str() != Some(token) {
        return Err("Conflict or file changed; reload before saving".into());
    }
    let abs = safe_path(root, path)?;
    if before["binary"].as_bool() == Some(true) && choice.is_none() {
        return Err("Choose a whole version for a binary conflict".into());
    }
    let value = match choice {
        Some("current") => blob(root, &format!(":2:{path}"))?,
        Some("incoming") => blob(root, &format!(":3:{path}"))?,
        Some("delete") => None,
        Some(_) => return Err("Invalid conflict choice".into()),
        None => Some(
            content
                .ok_or("Missing resolved content")?
                .as_bytes()
                .to_vec(),
        ),
    };
    if stage
        && choice.is_none()
        && content.is_some_and(|c| {
            c.lines()
                .any(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ") || l == "=======")
        })
    {
        return Err("Resolve every conflict marker before marking resolved".into());
    }
    match value {
        Some(b) => {
            std::fs::write(&abs, b).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            if let Some(stage) = match choice {
                Some("current") => Some("2"),
                Some("incoming") => Some("3"),
                _ => None,
            } {
                use std::os::unix::fs::PermissionsExt;
                let entries = text(root, &["ls-files", "--unmerged", "-z", "--", path])?;
                if let Some(mode) = entries.split('\0').find_map(|entry| {
                    let fields: Vec<_> = entry.split_whitespace().take(3).collect();
                    (fields.len() == 3 && fields[2] == stage).then(|| fields[0])
                }) {
                    let permissions = std::fs::Permissions::from_mode(if mode == "100755" {
                        0o755
                    } else {
                        0o644
                    });
                    std::fs::set_permissions(&abs, permissions).map_err(|e| e.to_string())?;
                }
            }
        }
        None => {
            if abs.exists() {
                std::fs::remove_file(abs).map_err(|e| e.to_string())?;
            }
        }
    }
    if stage {
        git(root, &["add", "--", path])?;
        Ok(json!({"ok":true}))
    } else {
        conflict(root, path)
    }
}
pub(super) fn blame(root: &Path, path: &str) -> Result<Value, String> {
    safe_path(root, path)?;
    let head = oid(root, "HEAD")?;
    let source = blob(root, &format!("{head}:{path}"))?;
    utf8(&source)?;
    let raw = text(root, &["blame", "--line-porcelain", &head, "--", path])?;
    let mut rows = Vec::new();
    let (mut hash, mut author, mut time, mut summary) =
        (String::new(), String::new(), String::new(), String::new());
    for l in raw.lines() {
        if let Some(t) = l.strip_prefix('\t') {
            rows.push(json!({"oid":hash,"author":author,"time":time,"summary":summary,"text":t}));
        } else if let Some(v) = l.strip_prefix("author ") {
            author = v.into()
        } else if let Some(v) = l.strip_prefix("author-time ") {
            time = v.into()
        } else if let Some(v) = l.strip_prefix("summary ") {
            summary = v.into()
        } else if l
            .split_whitespace()
            .next()
            .is_some_and(|s| s.len() >= 40 && s.chars().all(|c| c.is_ascii_hexdigit()))
        {
            hash = l.split_whitespace().next().unwrap_or_default().into();
        }
    }
    Ok(json!({"head":head,"rows":rows,"workingTreeDiffers":worktree(root,path)?!=source}))
}
