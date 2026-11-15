//! `usage.query` and the `session_info.usage` projection. The query itself is
//! `model::usage::query::run`; this module validates the params, maps
//! `workdir` to the ledger's bucket hash, runs the query off the async
//! runtime and joins session names from the registry.
use crate::app::runtime::sdk::{
    methods::HandlerFuture,
    protocol::{Reason, Request, RpcError},
    Adapter,
};
use crate::ipc::proto::SessionSnapshot;
use crate::model::usage::query::{
    self, GroupBy, LedgerState, RoleKind, UsageError, UsageGroup, UsageQuery, UsageReport,
    UsageTotals,
};
use crate::model::usage::BucketSize;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;

const MAX_ID_CHARS: usize = 200;

// ── DTOs ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Totals {
    pub calls: i64,
    pub tokens_in: i64,
    pub tokens_cached: i64,
    pub tokens_out: i64,
    pub cost: f64,
}

impl From<&UsageTotals> for Totals {
    fn from(t: &UsageTotals) -> Self {
        Self {
            calls: t.calls,
            tokens_in: t.tokens_in,
            tokens_cached: t.tokens_cached,
            tokens_out: t.tokens_out,
            cost: t.cost,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Group {
    pub key: String,
    pub label: Option<String>,
    #[serde(flatten)]
    pub totals: Totals,
    /// `group_by:"role"` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_kind: Option<&'static str>,
    /// `group_by:"role"` only; `null` for `main`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<Option<String>>,
    /// `group_by:"session"` only; `null` when the session is unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<Option<&'static str>>,
    /// `group_by:"session"` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Bucket {
    pub start_at: i64,
    #[serde(flatten)]
    pub totals: Totals,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Report {
    pub from_at: i64,
    pub to_at: i64,
    pub ledger: &'static str,
    pub totals: Totals,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<Group>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buckets: Option<Vec<Bucket>>,
}

/// The bound session's spend as its daemon last reported it. `tokens_out` and
/// `cost` are cumulative; the prompt and cache sizes are the latest call's.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SessionUsage {
    pub last_prompt_tokens: u64,
    pub last_cached_tokens: u64,
    pub output_tokens_total: u64,
    pub cost_total: f64,
    pub context: Option<ContextUsage>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ContextUsage {
    pub prompt_tokens: u64,
    pub effective_window: u64,
    pub estimated: bool,
    pub cached_tokens: Option<u64>,
    pub drss_active: bool,
}

impl SessionUsage {
    pub fn from_snapshot(s: &SessionSnapshot) -> Self {
        Self {
            last_prompt_tokens: s.tokens_in,
            last_cached_tokens: s.tokens_cached,
            output_tokens_total: s.tokens_out,
            cost_total: s.cost,
            context: s.context_usage.as_ref().map(|c| ContextUsage {
                prompt_tokens: c.prompt_tokens,
                effective_window: c.effective_window,
                estimated: c.estimated,
                cached_tokens: c.cached_tokens,
                drss_active: c.drss_active,
            }),
        }
    }
}

// ── params ─────────────────────────────────────────────────────────────────

fn out_of_range(message: impl Into<String>) -> RpcError {
    RpcError::reason(Reason::OutOfRange, message)
}

fn int_param(map: &Map<String, Value>, key: &str) -> Result<Option<i64>, RpcError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| out_of_range(format!("{key} must be an integer"))),
    }
}

fn str_param<'a>(map: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, RpcError> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(out_of_range(format!("{key} must be a string"))),
    }
}

/// Parse and bound `usage.query` params. The window, bucket count and group
/// limit are checked again by the query itself; the adapter checks the shapes.
pub fn parse(params: Value) -> Result<UsageQuery, RpcError> {
    let Value::Object(map) = params else {
        return Err(RpcError::reason(Reason::OutOfRange, "params must be an object"));
    };
    let session_id = match str_param(&map, "session_id")? {
        None => None,
        Some(id) if !id.is_empty() && id.chars().count() <= MAX_ID_CHARS => Some(id.to_string()),
        Some(_) => {
            return Err(out_of_range(format!(
                "session_id must be a string of 1 to {MAX_ID_CHARS} characters"
            )))
        }
    };
    let workdir = match str_param(&map, "workdir")? {
        None => None,
        Some(dir) => {
            let path = PathBuf::from(dir);
            if !path.is_absolute() {
                return Err(out_of_range("workdir must be an absolute path"));
            }
            Some(path)
        }
    };
    let group_by = match str_param(&map, "group_by")? {
        None | Some("none") => GroupBy::None,
        Some("model") => GroupBy::Model,
        Some("role") => GroupBy::Role,
        Some("session") => GroupBy::Session,
        Some(_) => {
            return Err(out_of_range(
                "group_by must be \"none\", \"model\", \"role\" or \"session\"",
            ))
        }
    };
    let bucket = match str_param(&map, "bucket")? {
        None | Some("none") => None,
        Some("hour") => Some(BucketSize::Hour),
        Some("day") => Some(BucketSize::Day),
        Some("week") => Some(BucketSize::Week),
        Some(_) => {
            return Err(out_of_range(
                "bucket must be \"none\", \"hour\", \"day\" or \"week\"",
            ))
        }
    };
    let tz_offset_minutes = match int_param(&map, "tz_offset_minutes")? {
        None => None,
        Some(m) => Some(
            i32::try_from(m)
                .ok()
                .filter(|m| {
                    (query::TZ_OFFSET_MINUTES_MIN..=query::TZ_OFFSET_MINUTES_MAX).contains(m)
                })
                .ok_or_else(|| {
                    out_of_range(format!(
                        "tz_offset_minutes must be between {} and {}",
                        query::TZ_OFFSET_MINUTES_MIN,
                        query::TZ_OFFSET_MINUTES_MAX
                    ))
                })?,
        ),
    };
    let limit = match int_param(&map, "limit")? {
        None => None,
        Some(n) => Some(
            u32::try_from(n)
                .ok()
                .filter(|n| (1..=query::MAX_GROUP_LIMIT).contains(n))
                .ok_or_else(|| {
                    out_of_range(format!(
                        "limit must be between 1 and {}",
                        query::MAX_GROUP_LIMIT
                    ))
                })?,
        ),
    };
    Ok(UsageQuery {
        from_at: int_param(&map, "from_at")?,
        to_at: int_param(&map, "to_at")?,
        session_id,
        workdir,
        group_by,
        bucket,
        tz_offset_minutes,
        limit,
    })
}

/// The wire error of a failed query.
pub fn error(e: UsageError) -> RpcError {
    let message = e.to_string();
    match e {
        UsageError::RangeTooLarge(_) => {
            RpcError::reason(Reason::RangeTooLarge, message)
        }
        UsageError::OutOfRange(_) => out_of_range(message),
        UsageError::Timeout => RpcError::reason(Reason::QueryTimeout, message),
        UsageError::Unreadable(_) => RpcError::reason(Reason::IoError, message),
    }
}

// ── session names ──────────────────────────────────────────────────────────

/// What the registry knows of one session.
#[derive(Debug, Clone, PartialEq)]
pub struct Known {
    pub name: String,
    pub room: bool,
}

/// Every registered session by id. A registry that does not exist yet is
/// empty; it is never created here.
pub fn registry_names() -> Result<HashMap<String, Known>, RpcError> {
    let io = |e: &dyn std::fmt::Display| {
        RpcError::reason(Reason::IoError, format!("session registry: {e}"))
    };
    let rows = crate::model::session_registry::list_all_existing().map_err(|e| io(&e))?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let room = crate::model::store::session_dir(&row.pwd_hash, &row.uuid)
                .map(|dir| crate::room::room_db_path(&dir).is_file())
                .unwrap_or(false);
            (
                row.uuid,
                Known {
                    name: row.name,
                    room,
                },
            )
        })
        .collect())
}

// ── projection ─────────────────────────────────────────────────────────────

fn group(g: &UsageGroup, by: GroupBy, names: &HashMap<String, Known>) -> Group {
    let mut out = Group {
        key: g.key.clone(),
        label: None,
        totals: Totals::from(&g.totals),
        role_kind: None,
        agent: None,
        kind: None,
        deleted: None,
    };
    match by {
        GroupBy::Role => {
            out.role_kind = g.role_kind.map(|k| match k {
                RoleKind::Main => "main",
                RoleKind::Sub => "sub",
            });
            out.agent = Some(g.agent.clone());
            out.label = g.agent.clone();
        }
        GroupBy::Session => match names.get(&g.key) {
            Some(known) => {
                out.label = Some(known.name.clone());
                out.kind = Some(Some(if known.room { "room" } else { "coding" }));
                out.deleted = Some(false);
            }
            None => {
                out.kind = Some(None);
                out.deleted = Some(true);
            }
        },
        GroupBy::None | GroupBy::Model => {}
    }
    out
}

/// Project a core report. `names` is only consulted for session groups.
pub fn project(report: &UsageReport, by: GroupBy, names: &HashMap<String, Known>) -> Report {
    Report {
        from_at: report.from_at,
        to_at: report.to_at,
        ledger: match report.ledger {
            LedgerState::Ok => "ok",
            LedgerState::Missing => "missing",
        },
        totals: Totals::from(&report.totals),
        groups: report
            .groups
            .as_ref()
            .map(|groups| groups.iter().map(|g| group(g, by, names)).collect()),
        groups_truncated: report.groups.as_ref().map(|_| report.groups_truncated),
        buckets: report.buckets.as_ref().map(|buckets| {
            buckets
                .iter()
                .map(|b| Bucket {
                    start_at: b.start_at,
                    totals: Totals::from(&b.totals),
                })
                .collect()
        }),
    }
}

/// Run `query` with `run`, then join session names from `names` (read only
/// when there are session groups to label).
pub fn execute(
    query: &UsageQuery,
    run: impl FnOnce(&UsageQuery) -> Result<UsageReport, UsageError>,
    names: impl FnOnce() -> Result<HashMap<String, Known>, RpcError>,
) -> Result<Value, RpcError> {
    let report = run(query).map_err(error)?;
    let joined = match (query.group_by, &report.groups) {
        (GroupBy::Session, Some(groups)) if !groups.is_empty() => names()?,
        _ => HashMap::new(),
    };
    serde_json::to_value(project(&report, query.group_by, &joined))
        .map_err(|e| RpcError::new("internal", e.to_string()))
}

// ── handler ────────────────────────────────────────────────────────────────

pub fn handle_query(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(async move {
        let query = parse(r.params)?;
        adapter
            .blocking(move || execute(&query, query::run, registry_names))
            .await
    })
}
