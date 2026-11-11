//! Turning a target request into a confirmed scope record.
//!
//! The model asks for network targets through `sec_runtime` (`op=up|net` with
//! `targets`). That request is the only thing it can do: the record that admits a
//! networked `sec_run` is written here, and only after the user answers the
//! approval prompt. A request the record already covers needs no second prompt.

use crate::app::sec::scope::{self as record, Caps, Host, Mode, Scope, State, Window};
use crate::tool::scope::ScopeBase;
use crate::tool::ToolCtx;

/// Who confirmed a scope written from the approval prompt.
pub const CONFIRMED_BY_USER: &str = "user";

/// The targets a `sec_runtime` call asks to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub targets: Vec<Host>,
    pub exclusions: Vec<Host>,
}

/// Parse the target request out of a call.
///
/// `Ok(None)` when the call opens nothing (another tool, another op, `mode=off`,
/// or an empty list). `Err` names the first cell that cannot be a host, so the
/// user is never asked to confirm a list that differs from what was parsed.
pub fn requested(tool: &str, args: &serde_json::Value) -> Result<Option<Request>, String> {
    if tool != "sec_runtime" {
        return Ok(None);
    }
    let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
    if !matches!(op, "up" | "net") {
        return Ok(None);
    }
    if op == "net" && args.get("mode").and_then(|v| v.as_str()) == Some("off") {
        return Ok(None);
    }
    let targets = cells(args, "targets")?;
    if targets.is_empty() {
        return Ok(None);
    }
    Ok(Some(Request {
        targets,
        exclusions: cells(args, "exclusions")?,
    }))
}

fn cells(args: &serde_json::Value, key: &str) -> Result<Vec<Host>, String> {
    let Some(list) = args.get(key).and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    let mut out: Vec<Host> = Vec::new();
    for item in list {
        let raw = item.as_str().unwrap_or("").trim();
        let cell = parse_cell(raw).ok_or_else(|| {
            format!("{key}: {raw:?} is not a host or host:port (no scheme, no path)")
        })?;
        if !out.contains(&cell) {
            out.push(cell);
        }
    }
    out.sort();
    Ok(out)
}

/// `host`, `host:port` or `host:port,port`. The comma form is what the egress
/// proxy's rule line takes, so it is accepted here rather than refused.
fn parse_cell(raw: &str) -> Option<Host> {
    if raw.is_empty() || raw.contains("://") || raw.contains('/') {
        return None;
    }
    if let Some((host, ports)) = raw.rsplit_once(':') {
        if ports.contains(',') {
            let mut cell = Host::parse(host)?;
            if !cell.ports.is_empty() {
                return None;
            }
            for p in ports.split(',') {
                cell.ports.push(p.trim().parse().ok()?);
            }
            cell.ports.sort_unstable();
            cell.ports.dedup();
            return Some(cell);
        }
    }
    Host::parse(raw)
}

/// Is the request already inside the confirmed, active scope?
///
/// Every requested target must be a declared cell. Exclusions only narrow, so a
/// request may carry more of them than the record does, never fewer.
#[must_use]
pub fn covered(scope: Option<&Scope>, request: &Request) -> bool {
    let Some(scope) = scope else {
        return false;
    };
    if scope.state != State::Active || scope.confirmed_by.is_none() {
        return false;
    }
    request.targets.iter().all(|t| scope.targets.contains(t))
        && scope
            .exclusions
            .iter()
            .all(|e| request.exclusions.contains(e))
}

/// The scope for this call's run, if one is on disk.
#[must_use]
pub fn current(ctx: &ToolCtx) -> Option<Scope> {
    record::load(ScopeBase::of(ctx)?.dir())
}

/// Write the scope the user just confirmed. Replaces the previous one: what the
/// prompt listed is exactly what is admitted afterwards.
pub fn grant(ctx: &ToolCtx, request: &Request) -> std::io::Result<Scope> {
    let base = ScopeBase::of(ctx).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "this session has no directory to keep a scope in",
        )
    })?;
    let previous = record::load(base.dir());
    let scope = Scope {
        version: previous.as_ref().map_or(1, |s| s.version + 1),
        state: State::Active,
        auth_mode: previous.as_ref().map_or(Mode::None, |s| s.auth_mode),
        targets: request.targets.clone(),
        exclusions: request.exclusions.clone(),
        caps: previous.as_ref().map_or_else(Caps::default, |s| s.caps),
        window: Window::default(),
        confirmed_by: Some(CONFIRMED_BY_USER.to_string()),
    };
    record::store(base.dir(), &scope)?;
    Ok(scope)
}

/// The line shown in the approval prompt.
#[must_use]
pub fn prompt(request: &Request) -> String {
    let mut text = format!("allow network access to: {}", join(&request.targets));
    if !request.exclusions.is_empty() {
        text.push_str(&format!(" · except: {}", join(&request.exclusions)));
    }
    text
}

/// Cells back in the `host[:port,port]` form the daemon takes.
#[must_use]
pub fn wire(cells: &[Host]) -> Vec<String> {
    cells
        .iter()
        .map(|c| {
            if c.ports.is_empty() {
                c.host.clone()
            } else {
                let ports: Vec<String> = c.ports.iter().map(u16::to_string).collect();
                format!("{}:{}", c.host, ports.join(","))
            }
        })
        .collect()
}

fn join(cells: &[Host]) -> String {
    wire(cells).join(", ")
}

/// What the model is told when it asks for targets nobody confirmed.
pub const UNCONFIRMED: &str =
    "these targets are not in the confirmed scope — the user must approve them first";

#[cfg(test)]
#[path = "scope_grant_test.rs"]
mod tests;
