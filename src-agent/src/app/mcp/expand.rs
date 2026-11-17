//! `${VAR}` / `${VAR:-default}` expansion for MCP server entries, applied to an
//! in-memory copy at connect time so the stored config keeps the literal text.

use crate::model::app_config::McpServerEntry;

fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Expand `${NAME}`, `${NAME:-default}` and `${env:NAME}` in `input`.
/// `:-` uses the default when NAME is unset or empty. An unset NAME without a
/// default keeps the `${...}` text literally and is reported in `missing`, so
/// configs written before expansion existed keep working. Malformed or
/// unterminated `${` is kept literally; the default text is not itself expanded.
pub(super) fn expand_str(
    input: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    missing: &mut Vec<String>,
) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        out.push_str(rest.get(..start).unwrap_or(""));
        let after = rest.get(start + 2..).unwrap_or("");
        let Some(end) = after.find('}') else {
            out.push_str(rest.get(start..).unwrap_or(""));
            return out;
        };
        let inner = after.get(..end).unwrap_or("");
        let spec = inner.strip_prefix("env:").unwrap_or(inner);
        let (name, default) = match spec.split_once(":-") {
            Some((n, d)) => (n, Some(d)),
            None => (spec, None),
        };
        if is_name(name) {
            match (lookup(name), default) {
                (Some(v), _) if !v.is_empty() || default.is_none() => out.push_str(&v),
                (_, Some(d)) => out.push_str(d),
                _ => {
                    if !missing.iter().any(|m| m == name) {
                        missing.push(name.to_string());
                    }
                    out.push_str("${");
                    out.push_str(inner);
                    out.push('}');
                }
            }
            rest = after.get(end + 1..).unwrap_or("");
        } else {
            out.push_str("${");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// A copy of `server` with command, args, env values, url, header values and cwd expanded. Names
/// (env keys, server name) are never expanded. Returns the unset variable names
/// that had no default, deduplicated.
pub(super) fn expand_entry(
    server: &McpServerEntry,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> (McpServerEntry, Vec<String>) {
    let mut missing = Vec::new();
    let mut e = server.clone();
    e.command = expand_str(&server.command, lookup, &mut missing);
    e.args = server
        .args
        .iter()
        .map(|a| expand_str(a, lookup, &mut missing))
        .collect();
    e.env = server
        .env
        .iter()
        .map(|(k, v)| (k.clone(), expand_str(v, lookup, &mut missing)))
        .collect();
    e.url = expand_str(&server.url, lookup, &mut missing);
    e.headers = server
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), expand_str(v, lookup, &mut missing)))
        .collect();
    e.cwd = server
        .cwd
        .as_deref()
        .map(|c| expand_str(c, lookup, &mut missing));
    (e, missing)
}
