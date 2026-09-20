//! Structured identities preserve numeric versions, tiers and snapshot dates.
use super::CatalogModel;

pub(super) struct Match<'a> {
    pub model: Option<&'a CatalogModel>,
    pub method: &'static str,
    pub uncertain: bool,
    pub conservative_window: Option<u64>,
}

pub(super) fn vendor(endpoint: &str) -> Option<&'static str> {
    let host = url::Url::parse(endpoint)
        .ok()?
        .host_str()?
        .to_ascii_lowercase();
    if host == "chatgpt.com" || host == "api.openai.com" {
        Some("openai")
    } else if host == "api.anthropic.com" || host == "claude.ai" {
        Some("anthropic")
    } else if host == "api.x.ai" {
        Some("x-ai")
    } else if host == "api.deepseek.com" {
        Some("deepseek")
    } else {
        None
    }
}

fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut numeric = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if !c.is_alphanumeric() {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
        } else {
            if !word.is_empty() && numeric != c.is_numeric() {
                out.push(std::mem::take(&mut word));
            }
            numeric = c.is_numeric();
            word.push(c);
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

#[derive(PartialEq, Eq)]
struct Identity {
    vendor: Option<String>,
    numbers: Vec<String>,
    names: Vec<String>,
}
fn identity(raw: &str, hint: Option<&str>) -> Identity {
    let (author, name) = raw
        .split_once('/')
        .map_or((hint, raw), |(a, n)| (Some(a), n));
    let mut tokens = words(name);
    let mut author = author.map(str::to_ascii_lowercase);
    if tokens
        .first()
        .is_some_and(|s| matches!(s.as_str(), "openai" | "anthropic" | "google" | "deepseek"))
    {
        let label = tokens.remove(0);
        if author.is_none() {
            author = Some(label);
        }
    }
    let (numbers, mut names): (Vec<_>, Vec<_>) = tokens
        .into_iter()
        .partition(|s| s.chars().all(char::is_numeric));
    names.sort();
    Identity {
        vendor: author,
        numbers,
        names,
    }
}

fn compatible(a: &Identity, b: &Identity) -> bool {
    a.numbers == b.numbers
        && match (&a.vendor, &b.vendor) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
}

fn protected(s: &str) -> bool {
    matches!(
        s,
        "mini"
            | "nano"
            | "pro"
            | "max"
            | "flash"
            | "haiku"
            | "sonnet"
            | "opus"
            | "codex"
            | "spark"
            | "preview"
            | "thinking"
            | "instant"
            | "free"
            | "extended"
            | "long"
            | "latest"
    )
}

// At most one substitution/insertion/deletion, only in a long alphabetic name.
fn close_word(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if protected(a) || protected(b) || a.len().min(b.len()) < 5 {
        return false;
    }
    let a: Vec<_> = a.chars().collect();
    let b: Vec<_> = b.chars().collect();
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        if a.len() >= b.len() {
            i += 1;
        }
        if b.len() >= a.len() {
            j += 1;
        }
    }
    edits + (a.len() - i) + (b.len() - j) <= 1
}

pub(super) fn find<'a>(query: &str, endpoint: &str, models: &'a [CatalogModel]) -> Match<'a> {
    let query = query.trim();
    let exact: Vec<_> = models
        .iter()
        .filter(|m| {
            m.id.eq_ignore_ascii_case(query)
                || (!m.canonical_slug.is_empty() && m.canonical_slug.eq_ignore_ascii_case(query))
        })
        .collect();
    if exact.len() == 1 {
        return Match {
            model: Some(exact[0]),
            method: "exact",
            uncertain: false,
            conservative_window: None,
        };
    }
    if exact.len() > 1 {
        return Match {
            model: None,
            method: "ambiguous",
            uncertain: true,
            conservative_window: exact.iter().filter_map(|m| m.window()).min(),
        };
    }
    let wanted = identity(query, vendor(endpoint));
    let mut normalized = Vec::new();
    let mut fuzzy = Vec::new();
    // Identity matching is independent of capability completeness: a model can
    // report an output limit even when its context length is absent.
    for model in models {
        let id = identity(&model.id, None);
        if !compatible(&wanted, &id) {
            continue;
        }
        // Never discard catalog/routing variant suffixes globally.
        if wanted.names == id.names {
            normalized.push(model);
            continue;
        }
        if wanted.names.len() == id.names.len()
            && wanted
                .names
                .iter()
                .zip(&id.names)
                .all(|(a, b)| close_word(a, b))
        {
            let changed = wanted
                .names
                .iter()
                .zip(&id.names)
                .filter(|(a, b)| a != b)
                .count();
            if changed == 1 {
                fuzzy.push(model);
            }
        }
    }
    let (candidates, method, uncertain) = if !normalized.is_empty() {
        (normalized, "normalized", false)
    } else {
        (fuzzy, "fuzzy_estimate", true)
    };
    if candidates.len() == 1 {
        return Match {
            model: Some(candidates[0]),
            method,
            uncertain,
            conservative_window: None,
        };
    }
    Match {
        model: None,
        method: if candidates.is_empty() {
            "unmatched"
        } else {
            "ambiguous"
        },
        uncertain: true,
        conservative_window: candidates.iter().filter_map(|m| m.window()).min(),
    }
}
