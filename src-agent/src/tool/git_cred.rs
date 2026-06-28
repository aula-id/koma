//! The `git_cred` tool: list / select the SSH identity key for git-over-SSH.
//!
//! This tool NEVER reads key file contents. It only inspects `~/.ssh` to find
//! identity keys (a file is an identity key when a sibling `<name>.pub` exists)
//! and returns bare FILENAMES — no paths, no key material. The "select" action
//! returns a tagged result that the runtime intercepts to write the chosen key
//! into the session's settings (so `git_operator` can inject `-i ~/.ssh/<key>`
//! into its SSH commands without the model ever seeing key contents).

use anyhow::Result;
use serde_json::{json, Value};

use super::{Tool, ToolCtx};

/// Sentinel prefix on a successful `select` result. The runtime's `git_cred`
/// interception recognises this, strips it to recover the bare key filename,
/// writes `settings.git_ssh_key = Some(key)`, and persists the session. The
/// model never sees it — the interception replaces it with a human-readable
/// confirmation. A `list` result or any `error:` result has no such prefix.
pub const GIT_CRED_SELECT_PREFIX: &str = "__git_cred_select__::";

/// List or select the SSH identity key used by this session for git-over-SSH.
pub struct GitCred;

impl Tool for GitCred {
    fn name(&self) -> &'static str {
        "git_cred"
    }

    fn description(&self) -> &'static str {
        "List the SSH identity keys available in ~/.ssh and select which one \
         this session uses for git-over-SSH. Key file contents are NEVER read \
         or returned — only filenames are used. Use action=\"list\" to see \
         available keys (the currently-selected one is marked). Use \
         action=\"select\" with key=\"<filename>\" (e.g. \"id_ed25519\") to \
         choose a key; the session will use it for subsequent git operations."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "select"],
                    "description": "\"list\" enumerates available identity keys; \"select\" sets the active key."
                },
                "key": {
                    "type": "string",
                    "description": "The bare filename of the identity key to select (e.g. \"id_ed25519\"). Required when action=\"select\"."
                }
            },
            "required": ["action"]
        })
    }

    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'action'"))?;

        // Resolve ~/.ssh via the same dirs crate the rest of the codebase uses.
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("cannot resolve home directory"))?;
        let ssh_dir = home.join(".ssh");

        match action {
            "list" => list_keys(&ssh_dir, &ctx.ssh_key),
            "select" => {
                let key = args
                    .get("key")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing required string argument 'key' for action=\"select\""))?;
                select_key(&ssh_dir, key)
            }
            other => Ok(format!("error: unknown action '{other}'; expected \"list\" or \"select\"")),
        }
    }
}

/// Enumerate identity keys in `ssh_dir` (only files that have a `.pub` sibling)
/// and return a newline-separated list. The currently-selected key (if any) is
/// annotated with `(selected)`. Key file contents are NEVER opened.
fn list_keys(ssh_dir: &std::path::Path, current: &Option<String>) -> Result<String> {
    if !ssh_dir.exists() {
        return Ok("~/.ssh does not exist; no identity keys found".into());
    }

    let mut keys: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(ssh_dir)
        .map_err(|e| anyhow::anyhow!("cannot read ~/.ssh: {e}"))?;

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = match file_name.to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };

        // Skip .pub files, known_hosts*, config, authorized_keys*, and
        // any entry that is a directory. We identify identity keys purely by
        // the existence of a matching <name>.pub sibling — no file read.
        if name.ends_with(".pub")
            || name.starts_with("known_hosts")
            || name == "config"
            || name.starts_with("authorized_keys")
        {
            continue;
        }
        // Skip directories.
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ft.is_dir() {
            continue;
        }

        // Only include files that have a .pub sibling (metadata only, no read).
        let pub_sibling = ssh_dir.join(format!("{name}.pub"));
        if !pub_sibling.exists() {
            continue;
        }

        // Annotate the currently-selected key.
        let label = if current.as_deref() == Some(name.as_str()) {
            format!("* {name} (selected)")
        } else {
            format!("  {name}")
        };
        keys.push(label);
    }

    if keys.is_empty() {
        return Ok("no identity keys found in ~/.ssh (no file with a matching .pub sibling)".into());
    }

    keys.sort();
    Ok(keys.join("\n"))
}

/// Validate `key` (reject path traversal), verify both `~/.ssh/<key>` and
/// `~/.ssh/<key>.pub` exist (metadata only), and on success return the tagged
/// result string so the runtime can persist the selection.
fn select_key(ssh_dir: &std::path::Path, key: &str) -> Result<String> {
    // Reject empty key name and any path traversal attempt in the bare filename.
    if key.is_empty() {
        return Ok("error: invalid key name (must not be empty)".into());
    }
    if key.contains('/') || key.contains('\\') || key.contains("..") {
        return Ok("error: invalid key name (must be a bare filename, no path separators or '..')".into());
    }
    // Reject whitespace and shell-special characters: the key name is embedded
    // inside a single-quoted ssh command string, so a single-quote in the name
    // would escape out of the quoting, and whitespace would cause word-splitting.
    if key.chars().any(|c| matches!(c, ' ' | '\t' | '\'' | '"' | '$' | '`')) {
        return Ok("error: invalid key name (must not contain whitespace or shell-special characters)".into());
    }

    let private_path = ssh_dir.join(key);
    let pub_path = ssh_dir.join(format!("{key}.pub"));

    // Check existence via metadata only — never open the file.
    if std::fs::metadata(&private_path).is_err() {
        return Ok(format!("error: ~/.ssh/{key} does not exist"));
    }
    if std::fs::metadata(&pub_path).is_err() {
        return Ok(format!("error: ~/.ssh/{key}.pub does not exist"));
    }

    // Return the tagged result for the runtime to apply.
    Ok(format!("{GIT_CRED_SELECT_PREFIX}{key}"))
}
