//! Bounded, linear-time project problem matchers; parsing happens off the webview.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Matcher {
    pattern: String,
    file: usize,
    line: usize,
    #[serde(default)]
    column: Option<usize>,
    #[serde(default)]
    message: Option<usize>,
}
pub(super) fn compile(items: &[Matcher]) -> Result<Vec<(regex::Regex, Matcher)>> {
    anyhow::ensure!(items.len() <= 16, "At most 16 task problem matchers");
    items
        .iter()
        .map(|m| {
            anyhow::ensure!(
                m.pattern.len() <= 4096,
                "Problem matcher exceeds 4096 bytes"
            );
            let regex = regex::RegexBuilder::new(&m.pattern)
                .size_limit(1024 * 1024)
                .build()
                .context("Invalid problem matcher (Rust regex syntax)")?;
            for index in [Some(m.file), Some(m.line), m.column, m.message]
                .into_iter()
                .flatten()
            {
                anyhow::ensure!(
                    index > 0 && index < regex.captures_len(),
                    "Problem matcher capture group is missing"
                );
            }
            Ok((regex, m.clone()))
        })
        .collect()
}
#[derive(Default)]
pub(super) struct Collector {
    active: Vec<(regex::Regex, Matcher)>,
    ansi: Option<regex::Regex>,
    cwd: PathBuf,
    pending: BTreeMap<String, String>,
    pub values: Vec<Value>,
}
impl Collector {
    pub fn start(&mut self, cwd: &Path, matchers: &[Matcher]) -> Result<()> {
        let ansi =
            regex::Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]").context("Compile ANSI escape matcher")?;
        self.active = compile(matchers)?;
        self.ansi = Some(ansi);
        self.cwd = cwd.to_path_buf();
        self.pending.clear();
        Ok(())
    }
    pub fn ingest(&mut self, stream: &str, text: &str) {
        if self.active.is_empty() || self.values.len() >= 200 {
            return;
        }
        let buffer = self.pending.entry(stream.into()).or_default();
        buffer.push_str(text);
        let mut lines = Vec::new();
        while let Some(i) = buffer.find('\n') {
            let line = buffer[..i].to_string();
            buffer.drain(..=i);
            lines.push(line);
        }
        if buffer.len() > 65536 {
            buffer.clear();
        }
        for line in lines {
            self.line(&line);
        }
    }
    pub fn finish(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        for line in pending.values() {
            self.line(line);
        }
    }
    fn line(&mut self, line: &str) {
        if self.values.len() >= 200 || line.len() > 65536 {
            return;
        }
        let Some(ansi) = &self.ansi else {
            return;
        }; // Collector has not been started.
        let clean = ansi.replace_all(line, "");
        for (regex, matcher) in &self.active {
            let Some(found) = regex.captures(&clean) else {
                continue;
            };
            let get = |i: usize| found.get(i).map(|m| m.as_str());
            let Some(file) = get(matcher.file).filter(|s| s.len() <= 32768) else {
                continue;
            };
            let Some(line) = get(matcher.line)
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|n| *n > 0)
            else {
                continue;
            };
            let column = matcher
                .column
                .and_then(get)
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(1)
                .max(1);
            let path = self.cwd.join(file).to_string_lossy().into_owned();
            if self
                .values
                .iter()
                .any(|p| p["path"] == path && p["line"] == line && p["column"] == column)
            {
                continue;
            }
            self.values.push(json!({"path":path,"line":line,"column":column,"message":matcher.message.and_then(get).unwrap_or(&clean).chars().take(2048).collect::<String>()}));
            if self.values.len() >= 200 {
                break;
            }
        }
    }
}
