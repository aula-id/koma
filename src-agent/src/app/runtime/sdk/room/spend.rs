//! `room.spend`: what each goal cost by the room's own spend notes, and what
//! the room cost by the global usage ledger.
//!
//! The goal part reads every `notes/spend/<goal>` note through the binding's
//! read-only shelf. The ledger part is one usage query for the room's session
//! id grouped by role, where `main` is the Chief and `sub:<avatar>` an avatar.
use super::reads::{goal_param, unreadable};
use super::{bound_room, object};
use crate::app::runtime::sdk::host::usage::{self, Totals};
use crate::app::runtime::sdk::methods::HandlerFuture;
use crate::app::runtime::sdk::protocol::{Request, RpcError};
use crate::app::runtime::sdk::Adapter;
use crate::model::usage::query::{
    GroupBy, LedgerState, RoleKind, UsageError, UsageQuery, UsageReport, MAX_RANGE_SECS,
};
use crate::room::ledger::{self, Entry};
use crate::room::shelf::Shelf;
use serde::Serialize;
use serde_json::Value;

/// `room.spend` caps.
const GOALS: usize = 200;
const BY_AVATAR: usize = 64;
const SPEND_PREFIX: &str = "notes/spend/";

#[derive(Debug, Clone, Serialize, PartialEq)]
struct SpendEntry {
    calls: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    cost: f64,
}

impl From<&Entry> for SpendEntry {
    fn from(e: &Entry) -> Self {
        Self {
            calls: e.calls,
            prompt_tokens: e.prompt_tokens,
            completion_tokens: e.completion_tokens,
            cost: e.cost.max(0.0),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct AvatarSpend {
    avatar: String,
    #[serde(flatten)]
    entry: SpendEntry,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct GoalSpend {
    goal_key: String,
    total: SpendEntry,
    by_avatar: Vec<AvatarSpend>,
    by_avatar_truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct LedgerAvatar {
    avatar: String,
    totals: Totals,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct LedgerSpend {
    ledger: &'static str,
    totals: Totals,
    by_avatar: Vec<LedgerAvatar>,
    by_avatar_truncated: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
struct SpendResult {
    goals: Vec<GoalSpend>,
    goals_truncated: bool,
    ledger: LedgerSpend,
}

/// Every goal with a spend note, in key order, or only `goal`.
fn goal_spend(shelf: &Shelf, goal: Option<&str>) -> Result<(Vec<GoalSpend>, bool), RpcError> {
    let notes = shelf.scan(SPEND_PREFIX).map_err(unreadable)?;
    let mut goals = Vec::new();
    for note in &notes {
        let Some(key) = note.key.strip_prefix(SPEND_PREFIX) else {
            continue;
        };
        if key.is_empty() || goal.is_some_and(|g| g != key) {
            continue;
        }
        let book = ledger::load(shelf, key);
        let mut by_avatar: Vec<AvatarSpend> = book
            .by_avatar
            .iter()
            .map(|(avatar, entry)| AvatarSpend {
                avatar: avatar.clone(),
                entry: SpendEntry::from(entry),
            })
            .collect();
        let by_avatar_truncated = by_avatar.len() > BY_AVATAR;
        by_avatar.truncate(BY_AVATAR);
        goals.push(GoalSpend {
            goal_key: key.to_string(),
            total: SpendEntry::from(&book.total()),
            by_avatar,
            by_avatar_truncated,
        });
    }
    let truncated = goals.len() > GOALS;
    goals.truncate(GOALS);
    Ok((goals, truncated))
}

/// The ledger query for `room_id`: the longest window a query may cover,
/// grouped by role.
pub fn ledger_query(room_id: &str) -> UsageQuery {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    UsageQuery {
        // A minute short of the cap, so the window stays inside it whatever
        // second the query starts in.
        from_at: Some(now + 1 - (MAX_RANGE_SECS - 60)),
        session_id: Some(room_id.to_string()),
        group_by: GroupBy::Role,
        limit: Some(BY_AVATAR as u32),
        ..UsageQuery::default()
    }
}

/// `main` is the Chief; `sub:<avatar>` is that avatar.
fn avatar_of_role(key: &str, kind: Option<RoleKind>, agent: Option<&str>) -> String {
    match (kind, agent) {
        (Some(RoleKind::Main), _) => "chief".to_string(),
        (Some(RoleKind::Sub), Some(agent)) if !agent.is_empty() => agent.to_string(),
        _ => key.strip_prefix("sub:").unwrap_or(key).to_string(),
    }
}

fn ledger_spend(report: &UsageReport) -> LedgerSpend {
    let mut by_avatar: Vec<LedgerAvatar> = Vec::new();
    for group in report.groups.iter().flatten() {
        let avatar = avatar_of_role(&group.key, group.role_kind, group.agent.as_deref());
        if avatar.is_empty() {
            continue;
        }
        by_avatar.push(LedgerAvatar {
            avatar,
            totals: Totals::from(&group.totals),
        });
    }
    let by_avatar_truncated = report.groups_truncated || by_avatar.len() > BY_AVATAR;
    by_avatar.truncate(BY_AVATAR);
    LedgerSpend {
        ledger: match report.ledger {
            LedgerState::Ok => "ok",
            LedgerState::Missing => "missing",
        },
        totals: Totals::from(&report.totals),
        by_avatar,
        by_avatar_truncated,
    }
}

pub fn handle_spend(r: Request, adapter: &mut Adapter) -> HandlerFuture<'_> {
    Box::pin(spend(r.params, adapter))
}

async fn spend(params: Value, adapter: &mut Adapter) -> Result<Value, RpcError> {
    let map = object(params)?;
    let goal = goal_param(&map)?;
    let room = bound_room(adapter)?;
    let shelf = room.shelf()?;
    let query = ledger_query(room.session_id());
    let run = std::sync::Arc::clone(&room.env.usage);
    let result = adapter
        .blocking(move || {
            let (goals, goals_truncated) = goal_spend(&shelf, goal.as_deref())?;
            let report = run(&query).map_err(|e: UsageError| usage::error(e))?;
            Ok(SpendResult {
                goals,
                goals_truncated,
                ledger: ledger_spend(&report),
            })
        })
        .await?;
    let value =
        serde_json::to_value(&result).map_err(|e| RpcError::new("internal", e.to_string()))?;
    Ok(bound_room(adapter)?.scrub(value).await)
}
