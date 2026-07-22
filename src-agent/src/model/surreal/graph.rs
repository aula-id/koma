//! Graph edges for agent orchestration and tool-call tracking.
//!
//! Uses SurrealDB RELATE edges to model the agent → tool_call →
//! message provenance graph.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use super::core::{self, embed_one, open_db};

/// Tool call record returned by [`get_agent_calls`].
#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub args_summary: String,
    pub result_snippet: String,
    pub timestamp: i64,
}

/// Record that `agent_id` invoked a tool. Fire-and-forget.
pub fn record_tool_call(
    session_dir: &Path,
    agent_id: &str,
    tool_name: &str,
    args_summary: &str,
    result_snippet: &str,
) {
    let sd = session_dir.to_path_buf();
    let aid = agent_id.to_string();
    let tn = tool_name.to_string();
    let args = args_summary.to_string();
    let res = result_snippet.to_string();
    // One OS thread + one current-thread runtime — do not nest blocking_block.
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                crate::model::store::append_global_error_log(
                    "surreal::record_tool_call — runtime build failed",
                    &e.to_string(),
                );
                return;
            }
        };
        rt.block_on(record_tc_async(&sd, &aid, &tn, &args, &res));
    });
}

async fn record_tc_async(
    session_dir: &Path,
    agent_id: &str,
    tool_name: &str,
    args_summary: &str,
    result_snippet: &str,
) {
    let db = match open_db(session_dir).await {
        Ok(db) => db,
        Err(e) => {
            crate::model::store::append_error_log(
                session_dir,
                "surreal::record_tool_call — open_db",
                &e.to_string(),
            );
            return;
        }
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let emb = embed_one(args_summary);

    // Create agent node with explicit record ID for RELATE edges.
    let agent_rid = format!("agent:{agent_id}");
    if let Err(e) = db
        .query("CREATE type::thing($rid) CONTENT $data")
        .bind(("rid", agent_rid.clone()))
        .bind(("data", serde_json::json!({
            "agent_id": agent_id,
            "name": agent_id,
            "last_seen": now,
        })))
        .await
    {
        crate::model::store::append_error_log(
            session_dir,
            "surreal::record_tool_call — CREATE agent",
            &e.to_string(),
        );
    }

    // Create tool_call node with explicit record ID.
    let tc_rid = format!("tool_call:tc_{agent_id}_{tool_name}_{now}");
    if let Err(e) = db
        .query("CREATE type::thing($rid) CONTENT $data")
        .bind(("rid", tc_rid.clone()))
        .bind(("data", serde_json::json!({
            "tc_id": format!("tc:{agent_id}:{tool_name}:{now}"),
            "agent_id": agent_id,
            "tool_name": tool_name,
            "args_summary": args_summary,
            "result_snippet": result_snippet,
            "embedding": emb,
            "timestamp": now,
        })))
        .await
    {
        crate::model::store::append_error_log(
            session_dir,
            "surreal::record_tool_call — CREATE tool_call",
            &e.to_string(),
        );
    }

    // RELATE agent → tool_call via the called edge.
    if let Err(e) = db
        .query("RELATE type::thing($a)->called->type::thing($tc)")
        .bind(("a", agent_rid.clone()))
        .bind(("tc", tc_rid.clone()))
        .await
    {
        crate::model::store::append_error_log(
            session_dir,
            "surreal::record_tool_call — RELATE called",
            &e.to_string(),
        );
    }
}

/// Fetch all tool calls made by `agent_id`, newest first.
pub fn get_agent_calls(session_dir: &Path, agent_id: &str) -> Vec<ToolCallRecord> {
    let sd = session_dir.to_path_buf();
    let aid = agent_id.to_string();
    core::blocking_block(move || {
        let sd = sd.clone();
        let aid = aid.clone();
        async move {
            get_agent_calls_async(&sd, &aid)
                .await
                .unwrap_or_else(|e| {
                    crate::model::store::append_error_log(
                        &sd,
                        "surreal::get_agent_calls failed",
                        &e.to_string(),
                    );
                    Vec::new()
                })
        }
    })
    .unwrap_or_default()
}

async fn get_agent_calls_async(
    session_dir: &Path,
    agent_id: &str,
) -> anyhow::Result<Vec<ToolCallRecord>> {
    let db = open_db(session_dir).await?;

    let mut res = db
        .query(
            "SELECT tool_name, args_summary, result_snippet, timestamp
             FROM tool_call
             WHERE agent_id = $aid
             ORDER BY timestamp DESC
             LIMIT 100",
        )
        .bind(("aid", agent_id.to_string()))
        .await?;

    let names: Vec<String> = res.take("tool_name").unwrap_or_default();
    let args: Vec<String> = res.take("args_summary").unwrap_or_default();
    let results: Vec<String> = res.take("result_snippet").unwrap_or_default();
    let timestamps: Vec<i64> = res.take("timestamp").unwrap_or_default();
    let n = names
        .len()
        .min(args.len())
        .min(results.len())
        .min(timestamps.len());

    Ok((0..n)
        .map(|i| ToolCallRecord {
            tool_name: names[i].clone(),
            args_summary: args[i].clone(),
            result_snippet: results[i].clone(),
            timestamp: timestamps[i],
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn test_dir() -> PathBuf {
        std::env::temp_dir().join("koma_test_surreal_graph")
    }

    #[test]
    fn test_get_agent_calls_unknown_agent() {
        let dir = test_dir();
        let _ = fs::create_dir_all(&dir);
        let calls = get_agent_calls(&dir, "nonexistent");
        assert!(calls.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
