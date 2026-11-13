//! Skill search: score a query against name, description, triggers and tags.
//!
//! The model never sees a skill list; it asks for what it needs and gets a few hits back. The
//! scorer is plain word matching, so it is instant at thousands of skills and works offline.
//! A model re-rank can sit on top (see `decisive`), but the lexical result is always the fallback.

use super::def::SkillDef;

/// Words too common to say anything about a skill.
const STOP: &[&str] = &["the", "and", "for", "with", "that", "this", "from", "into", "use", "when", "how", "are", "you", "your"];

/// Which skill names a caller may see. Empty means all. A pattern is a name or a name with `*`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope(Vec<String>);

impl Scope {
    pub fn new(patterns: &[String]) -> Self {
        Self(patterns.iter().map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).collect())
    }

    pub fn allows(&self, name: &str) -> bool {
        self.0.is_empty() || self.0.iter().any(|p| glob(p, name))
    }
}

fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = text.strip_prefix(first) else { return false };
    if !pattern.contains('*') {
        return rest.is_empty();
    }
    let mut tail: Vec<&str> = parts.collect();
    let last = tail.pop().unwrap_or("");
    for mid in tail {
        match rest.find(mid) {
            Some(at) => rest = &rest[at + mid.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub name: String,
    pub score: u32,
}

pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.len() > 1 && !STOP.contains(&w.as_str()))
        .collect()
}

/// `tags` from the frontmatter, as a list or a comma separated string.
pub fn tags_of(def: &SkillDef) -> Vec<String> {
    let Some(map) = def.frontmatter.as_ref() else { return Vec::new() };
    match map.get("tags") {
        Some(serde_yaml_ng::Value::Sequence(items)) => items.iter().filter_map(|v| v.as_str().map(str::to_lowercase)).collect(),
        Some(serde_yaml_ng::Value::String(s)) => s.split(',').map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect(),
        _ => Vec::new(),
    }
}

/// `pinned: true` in the frontmatter keeps a skill visible in the prompt.
pub fn is_pinned(def: &SkillDef) -> bool {
    def.frontmatter.as_ref().and_then(|m| m.get("pinned")).and_then(serde_yaml_ng::Value::as_bool).unwrap_or(false)
}

fn word_score(query: &[String], field: &[String], exact: u32, prefix: u32, inside: u32, hit: &mut [bool]) -> u32 {
    let mut total = 0;
    for (i, q) in query.iter().enumerate() {
        let best = field
            .iter()
            .map(|f| {
                if f == q {
                    exact
                } else if f.starts_with(q.as_str()) || q.starts_with(f.as_str()) && f.len() > 3 {
                    prefix
                } else if f.contains(q.as_str()) {
                    inside
                } else {
                    0
                }
            })
            .max()
            .unwrap_or(0);
        if best > 0 {
            hit[i] = true;
        }
        total += best;
    }
    total
}

pub fn score(def: &SkillDef, query: &[String], whole: &str) -> u32 {
    if query.is_empty() {
        return 0;
    }
    let mut hit = vec![false; query.len()];
    let name = words(&def.name);
    let desc = words(&def.description);
    let trig = words(&def.triggers);
    let tags: Vec<String> = tags_of(def).iter().flat_map(|t| words(t)).collect();
    let mut s = word_score(query, &name, 10, 8, 6, &mut hit)
        + word_score(query, &desc, 3, 2, 1, &mut hit)
        + word_score(query, &trig, 5, 3, 2, &mut hit)
        + word_score(query, &tags, 5, 3, 2, &mut hit);
    if s == 0 {
        return 0;
    }
    if def.name == whole {
        s += 30;
    }
    s + 2 * hit.iter().filter(|h| **h).count() as u32
}

/// Whether the best hit is clear enough that a re-rank would not change the answer.
pub fn decisive(hits: &[Hit]) -> bool {
    match hits {
        [] => true,
        [_] => true,
        [a, b, ..] => a.score >= 12 && a.score >= b.score * 2,
    }
}
