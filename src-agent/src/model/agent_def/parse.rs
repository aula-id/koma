//! Frontmatter parsing helpers: split, find fence, deserialize, validate name,
//! and the file-level loader [`load_agent_file`].

use std::path::Path;

use anyhow::{anyhow, Result};

use super::def::{AgentDef, AgentSource};

// ---------------------------------------------------------------------------
// Frontmatter parsing
// ---------------------------------------------------------------------------

/// Split a `.md` file into its optional YAML frontmatter and markdown body.
///
/// Format: `---\n<yaml>\n---\n<body>`. A file that does NOT start with `---` is
/// treated as all body (no frontmatter), which is not an error. An opening `---`
/// with no closing delimiter IS an error.
pub(crate) fn split_frontmatter(content: &str) -> Result<(Option<&str>, &str)> {
    // Only treat a leading `---` followed by a newline as a frontmatter fence,
    // so a body that merely starts with a markdown horizontal rule (`---\ntext`)
    // is handled, but a Setext heading like "Title\n---" is not misread.
    let after_fence = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"));
    let Some(rest) = after_fence else {
        return Ok((None, content));
    };
    // Find the closing fence: a line that is exactly `---`.
    let Some((fm, body)) = find_closing_fence(rest) else {
        return Err(anyhow!("frontmatter not closed (missing closing ---)"));
    };
    Ok((Some(fm), body.trim_start_matches(['\n', '\r'])))
}

/// Locate the closing `---` fence line in `rest` (everything after the opening
/// fence). Returns `(frontmatter, body)` split around it, or `None` if absent.
fn find_closing_fence(rest: &str) -> Option<(&str, &str)> {
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" {
            let fm = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return Some((fm, body));
        }
        offset += line.len();
    }
    None
}

/// Deserialize frontmatter YAML into a partial [`AgentDef`].
///
/// All frontmatter fields are optional (`Option` or serde default), so missing
/// keys fall back to defaults; `name`/`prompt`/`source`/`file_path` stay at their
/// `Default` until the loader fills them in.
fn parse_frontmatter(yaml_str: &str) -> Result<AgentDef> {
    // An empty frontmatter block deserializes to the default agent.
    if yaml_str.trim().is_empty() {
        return Ok(AgentDef::default());
    }
    let agent: AgentDef = serde_yaml_ng::from_str(yaml_str)?;
    Ok(agent)
}

/// Validate an agent name derived from a filename stem.
///
/// Rules: lowercase ASCII alphanumeric + dash only; no path traversal (`/`,
/// `\`, `..`); no leading/trailing dash; non-empty.
pub(crate) fn validate_agent_name(stem: &str) -> Result<String> {
    let lower = stem.to_lowercase();

    if lower.contains('/') || lower.contains('\\') || lower.contains("..") {
        return Err(anyhow!("invalid agent name: path traversal detected"));
    }
    if lower.is_empty() {
        return Err(anyhow!("invalid agent name: empty"));
    }
    if !lower.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(anyhow!(
            "invalid agent name: only alphanumeric and dash allowed"
        ));
    }
    if lower.starts_with('-') || lower.ends_with('-') {
        return Err(anyhow!("invalid agent name: no leading/trailing dash"));
    }
    Ok(lower)
}

/// Parse an in-memory `.md` string into an [`AgentDef`].
///
/// Shared by [`load_agent_file`] and the unit tests. `name`/`source` are caller
/// supplied; `file_path` is `None` here (set by the file loader).
pub(crate) fn parse_agent(name: &str, content: &str, source: AgentSource) -> Result<AgentDef> {
    let (fm_str, body) = split_frontmatter(content)?;

    let mut agent = match fm_str {
        Some(fm) => parse_frontmatter(fm)?,
        None => AgentDef::default(),
    };

    agent.name = validate_agent_name(name)?;
    agent.source = source;
    agent.prompt = body.to_string();

    if agent.description.trim().is_empty() {
        return Err(anyhow!(
            "agent {} missing required 'description'",
            agent.name
        ));
    }
    Ok(agent)
}

/// Load and parse a single `.md` agent file from disk.
///
/// The name is taken from the filename stem (lowercased + validated); the body
/// becomes the prompt. Any error (IO, UTF-8, malformed YAML, missing
/// description, bad name) is returned so the caller can skip the file non-fatally.
pub fn load_agent_file(path: &Path, source: AgentSource) -> Result<AgentDef> {
    let content = std::fs::read_to_string(path)?;
    let stem = path
        .file_stem()
        .ok_or_else(|| anyhow!("no filename"))?
        .to_string_lossy()
        .into_owned();

    let mut agent = parse_agent(&stem, &content, source)?;
    agent.file_path = Some(path.to_path_buf());
    Ok(agent)
}
