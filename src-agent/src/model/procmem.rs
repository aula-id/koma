//! Best-effort resident-memory sample for the GUI usage readout.
//!
//! The GUI process owns the WebKit footprint, so this is sampled here rather
//! than on the session daemon's Status fingerprint. Linux reads `/proc`. The
//! macOS and Windows readers live next to the GUI event loop and call into
//! [`assemble_sample`] with the same rules.

use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Resident bytes grouped the way the usage card draws them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemSample {
    /// This GUI process plus its WebKit descendants.
    pub window: u64,
    /// The foreground session daemon, when its pid file names another process.
    pub agent: u64,
    /// MCP, OAuth, and linker daemons, skipping duplicates of the two above.
    pub services: u64,
    /// Physical memory. Zero when this OS has no reader.
    pub system: u64,
}

/// Token counters copied off a `Status` push. Absent fields keep the previous
/// value so an older host does not zero the menu bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveTokens {
    pub tokens_in: u64,
    pub tokens_cached: u64,
    pub tokens_out: u64,
    pub cost_micros: u64,
    pub context_window: u64,
    pub working: bool,
}

/// `VmRSS:` kilobytes from `/proc/<pid>/status`, as bytes.
pub fn parse_vm_rss_bytes(status: &str) -> Option<u64> {
    parse_kb_field(status, "VmRSS:")
}

/// `MemTotal:` kilobytes from `/proc/meminfo`, as bytes.
pub fn parse_mem_total_bytes(meminfo: &str) -> Option<u64> {
    parse_kb_field(meminfo, "MemTotal:")
}

/// Advisory pid-file body. `0` and non-numeric text are not pids.
pub fn parse_pid_file(text: &str) -> Option<u32> {
    let pid: u32 = text.trim().parse().ok()?;
    if pid == 0 {
        None
    } else {
        Some(pid)
    }
}

/// `Name:` from `/proc/<pid>/status`. Empty when the field is absent.
pub fn proc_status_name(status: &str) -> &str {
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Name:") {
            return rest.trim();
        }
    }
    ""
}

/// Process comms whose resident size belongs on the Koma card.
/// `/proc` truncates `comm` to 15 bytes, so WebKit names are prefixes.
pub fn is_koma_or_webkit(name: &str) -> bool {
    let name = name.trim();
    name == "koma" || name == "koma.bin" || name.starts_with("WebKit") || name.starts_with("webkit")
}

/// Dollars to integer micro-dollars for the C ABI. Non-finite and negative
/// costs are zero.
pub fn cost_to_micros(cost: f64) -> u64 {
    if !cost.is_finite() || cost <= 0.0 {
        return 0;
    }
    let micros = (cost * 1_000_000.0).round();
    if micros >= u64::MAX as f64 {
        return u64::MAX;
    }
    micros as u64
}

/// Fold one `Status` object into `tokens`. Returns false when `value` is not
/// that envelope. Missing counters stay as they were.
pub fn apply_status(tokens: &mut LiveTokens, value: &Value) -> bool {
    if value.get("k").and_then(|k| k.as_str()) != Some("Status") {
        return false;
    }
    if let Some(n) = json_u64(value, "tokensIn") {
        tokens.tokens_in = n;
    }
    if let Some(n) = json_u64(value, "tokensCached") {
        tokens.tokens_cached = n;
    }
    if let Some(n) = json_u64(value, "tokensOut") {
        tokens.tokens_out = n;
    }
    if let Some(cost) = value.get("cost").and_then(|v| v.as_f64()) {
        tokens.cost_micros = cost_to_micros(cost);
    }
    if let Some(n) = json_u64(value, "contextWindow") {
        tokens.context_window = n;
    }
    if let Some(working) = value.get("working").and_then(|v| v.as_bool()) {
        tokens.working = working;
    }
    true
}

/// Session id on a `Status` push.
pub fn status_session_id(value: &Value) -> Option<&str> {
    let id = value.get("session").and_then(|v| v.as_str())?;
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// 1 MiB buckets. The GUI enqueues a web push only when one of these moves,
/// so a single-page RSS flicker does not grow the push queue.
pub fn mem_mib_key(mem: &MemSample) -> (u64, u64, u64, u64) {
    let mib = |bytes: u64| bytes / (1024 * 1024);
    (
        mib(mem.window),
        mib(mem.agent),
        mib(mem.services),
        mib(mem.system),
    )
}

/// Provider uuid → display name from a `Config.providers` array.
pub fn index_provider_names(providers: Option<&Value>) -> HashMap<String, String> {
    index_named_rows(providers, "id", "name")
}

/// OAuth connection uuid → display name from an `OAuthState.conns` array.
pub fn index_oauth_names(conns: Option<&Value>) -> HashMap<String, String> {
    index_named_rows(conns, "uuid", "name")
}

/// One line per runtime role, in catalogue order. `routes` is a Snapshot
/// `modelRoutes` array (or null). The value is `Provider · model`, the model
/// id alone when the provider has no name, or an em dash when nothing is bound.
/// An inherited role still shows the model it will call.
pub fn role_usage_lines(
    routes: &Value,
    providers: &HashMap<String, String>,
    oauth: &HashMap<String, String>,
) -> Vec<(&'static str, String)> {
    const ROLES: &[(&str, &str)] = &[
        ("main", "Main"),
        ("awareness", "Awareness"),
        ("safeguard", "Safeguard"),
        ("compactor", "Compactor"),
        ("planner", "Planner"),
    ];
    let rows = routes.as_array();
    ROLES
        .iter()
        .map(|(key, label)| {
            let route = rows.and_then(|list| {
                list.iter()
                    .find(|row| row.get("role").and_then(Value::as_str) == Some(*key))
            });
            (*label, role_usage_value(route, providers, oauth))
        })
        .collect()
}

fn role_usage_value(
    route: Option<&Value>,
    providers: &HashMap<String, String>,
    oauth: &HashMap<String, String>,
) -> String {
    let Some(route) = route else {
        return "—".to_string();
    };
    let Some(model) =
        text_field(route, "effective_model").or_else(|| text_field(route, "configured_model"))
    else {
        return "—".to_string();
    };
    let uuid = text_field(route, "provider_uuid").unwrap_or("");
    match providers
        .get(uuid)
        .or_else(|| oauth.get(uuid))
        .map(String::as_str)
        .filter(|name| !name.is_empty())
    {
        Some(provider) => format!("{provider} · {model}"),
        None => model.to_string(),
    }
}

fn text_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn index_named_rows(list: Option<&Value>, id_key: &str, name_key: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(rows) = list.and_then(Value::as_array) else {
        return out;
    };
    for row in rows {
        let Some(id) = text_field(row, id_key) else {
            continue;
        };
        let Some(name) = text_field(row, name_key) else {
            continue;
        };
        out.insert(id.to_string(), name.to_string());
    }
    out
}

pub fn usage_live_json(mem: &MemSample) -> String {
    serde_json::json!({
        "k": "UsageLive",
        "memWindow": mem.window,
        "memAgent": mem.agent,
        "memServices": mem.services,
        "memSystem": mem.system,
    })
    .to_string()
}

/// Inputs for [`assemble_sample`] besides the pid readers.
pub struct AssembleInput<'a> {
    pub self_pid: u32,
    /// Resident bytes of this process. Not passed through the name filter.
    pub self_rss: u64,
    pub system: u64,
    pub agent_pid: Option<u32>,
    pub helper_pids: &'a [u32],
}

/// `rss` is resident bytes for a pid that is still Koma or WebKit, else 0.
/// `children` lists direct child pids. Agent and helper pids are not added to
/// the window, and a helper that is already the agent is counted once.
pub fn assemble_sample(
    input: AssembleInput<'_>,
    mut rss: impl FnMut(u32) -> u64,
    mut children: impl FnMut(u32) -> Vec<u32>,
) -> MemSample {
    let AssembleInput {
        self_pid,
        self_rss,
        system,
        agent_pid,
        helper_pids,
    } = input;
    let mut skip = HashSet::new();
    skip.insert(self_pid);
    if let Some(pid) = agent_pid {
        if pid != self_pid {
            skip.insert(pid);
        }
    }
    for pid in helper_pids {
        if *pid != 0 && *pid != self_pid {
            skip.insert(*pid);
        }
    }

    let mut window = self_rss;
    let mut seen = HashSet::new();
    seen.insert(self_pid);
    let mut stack: Vec<(u32, u8)> = children(self_pid).into_iter().map(|pid| (pid, 1)).collect();
    while let Some((pid, depth)) = stack.pop() {
        if depth > 4 || pid == 0 || !seen.insert(pid) {
            continue;
        }
        if skip.contains(&pid) {
            continue;
        }
        window = window.saturating_add(rss(pid));
        if depth < 4 {
            for child in children(pid) {
                stack.push((child, depth + 1));
            }
        }
    }

    let agent = match agent_pid {
        Some(pid) if pid != self_pid => rss(pid),
        _ => 0,
    };

    let mut services = 0u64;
    let mut counted = HashSet::new();
    counted.insert(self_pid);
    if let Some(pid) = agent_pid {
        counted.insert(pid);
    }
    for pid in helper_pids {
        if *pid != 0 && counted.insert(*pid) {
            services = services.saturating_add(rss(*pid));
        }
    }

    MemSample {
        window,
        agent,
        services,
        system,
    }
}

fn parse_kb_field(text: &str, key: &str) -> Option<u64> {
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix(key) else {
            continue;
        };
        let Some(kb) = rest
            .split_whitespace()
            .next()
            .and_then(|n| n.parse::<u64>().ok())
        else {
            continue;
        };
        return Some(kb.saturating_mul(1024));
    }
    None
}

fn json_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|v| v.as_u64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vm_rss_and_mem_total_parse_as_bytes() {
        let status = "\
Name:\tkoma\n\
VmPeak:\t  99999 kB\n\
VmRSS:\t   1234 kB\n\
VmHWM:\t   2000 kB\n";
        assert_eq!(parse_vm_rss_bytes(status), Some(1234 * 1024));
        assert_eq!(proc_status_name(status), "koma");
        let info = "\
MemFree:        100 kB\n\
MemTotal:     16384000 kB\n\
MemAvailable:  8000000 kB\n";
        assert_eq!(parse_mem_total_bytes(info), Some(16_384_000 * 1024));
        assert_eq!(parse_vm_rss_bytes("no rss here"), None);
    }

    #[test]
    fn pid_file_rejects_zero_and_junk() {
        assert_eq!(parse_pid_file("4321\n"), Some(4321));
        assert_eq!(parse_pid_file("0"), None);
        assert_eq!(parse_pid_file("nope"), None);
        assert_eq!(parse_pid_file("12 34"), None);
    }

    #[test]
    fn comm_filter_keeps_koma_and_webkit_prefixes() {
        assert!(is_koma_or_webkit("koma"));
        assert!(is_koma_or_webkit("koma.bin"));
        assert!(is_koma_or_webkit("WebKitWebProces"));
        assert!(is_koma_or_webkit("WebKitNetworkPr"));
        assert!(!is_koma_or_webkit("firefox"));
        assert!(!is_koma_or_webkit(""));
    }

    #[test]
    fn cost_micros_drops_bad_values() {
        assert_eq!(cost_to_micros(0.0123456), 12_346);
        assert_eq!(cost_to_micros(0.0), 0);
        assert_eq!(cost_to_micros(-1.0), 0);
        assert_eq!(cost_to_micros(f64::NAN), 0);
        assert_eq!(cost_to_micros(f64::INFINITY), 0);
    }

    #[test]
    fn status_keeps_fields_the_payload_omits() {
        let mut tokens = LiveTokens {
            tokens_in: 10,
            tokens_cached: 4,
            tokens_out: 2,
            cost_micros: 5,
            context_window: 1000,
            working: true,
        };
        let partial =
            serde_json::json!({"k":"Status","session":"abc","working":false,"tokensIn":80});
        assert!(apply_status(&mut tokens, &partial));
        assert_eq!(tokens.tokens_in, 80);
        assert_eq!(tokens.tokens_cached, 4);
        assert_eq!(tokens.context_window, 1000);
        assert!(!tokens.working);
        assert_eq!(status_session_id(&partial), Some("abc"));
        assert!(!apply_status(&mut tokens, &serde_json::json!({"k":"Hub"})));
        assert_eq!(tokens.tokens_in, 80);
    }

    #[test]
    fn status_cost_and_window_land() {
        let mut tokens = LiveTokens::default();
        let full = serde_json::json!({
            "k": "Status",
            "tokensIn": 42000u64,
            "tokensCached": 1000u64,
            "tokensOut": 300u64,
            "cost": 1.25,
            "contextWindow": 200000u64,
            "working": true,
        });
        assert!(apply_status(&mut tokens, &full));
        assert_eq!(tokens.tokens_in, 42_000);
        assert_eq!(tokens.cost_micros, 1_250_000);
        assert_eq!(tokens.context_window, 200_000);
        assert!(tokens.working);
    }

    #[test]
    fn assemble_splits_window_agent_services_and_webkit_children() {
        // self 1, webkit child 2, unrelated child 3, nested webkit 4 under a
        // stub 5, agent 6 (also a child — must not join the window), helper 7,
        // duplicate helper 6.
        let rss = |pid: u32| match pid {
            2 => 5_000,
            4 => 50,
            6 => 8_000,
            7 => 100,
            _ => 0,
        };
        let children = |pid: u32| match pid {
            1 => vec![2, 3, 5, 6],
            5 => vec![4],
            _ => Vec::new(),
        };
        let sample = assemble_sample(
            AssembleInput {
                self_pid: 1,
                self_rss: 1_000,
                system: 16_000,
                agent_pid: Some(6),
                helper_pids: &[7, 6, 7],
            },
            rss,
            children,
        );
        assert_eq!(sample.window, 1_000 + 5_000 + 50);
        assert_eq!(sample.agent, 8_000);
        assert_eq!(sample.services, 100);
        assert_eq!(sample.system, 16_000);
    }

    #[test]
    fn assemble_skips_self_pid_as_agent_or_helper() {
        let sample = assemble_sample(
            AssembleInput {
                self_pid: 9,
                self_rss: 400,
                system: 0,
                agent_pid: Some(9),
                helper_pids: &[9, 0],
            },
            |_| 999,
            |_| vec![9],
        );
        assert_eq!(sample.window, 400);
        assert_eq!(sample.agent, 0);
        assert_eq!(sample.services, 0);
    }

    #[test]
    fn mib_key_ignores_sub_mebibyte_noise() {
        let mut mem = MemSample {
            window: 10 * 1024 * 1024 + 4096,
            agent: 1024,
            services: 0,
            system: 1024 * 1024 * 1024,
        };
        assert_eq!(mem_mib_key(&mem), (10, 0, 0, 1024));
        mem.window += 100;
        assert_eq!(mem_mib_key(&mem), (10, 0, 0, 1024));
        let parsed: serde_json::Value = serde_json::from_str(&usage_live_json(&mem)).unwrap();
        assert_eq!(parsed["k"], "UsageLive");
        assert_eq!(parsed["memWindow"], mem.window);
        assert_eq!(parsed["memAgent"], 1024);
    }

    #[test]
    fn role_lines_name_the_model_and_provider() {
        let routes = serde_json::json!([
            {
                "role": "main",
                "effective_model": "anthropic/claude-sonnet-4.5",
                "configured_model": "anthropic/claude-sonnet-4.5",
                "provider_uuid": "p1",
                "origin": "session"
            },
            {
                "role": "awareness",
                "effective_model": "openai/gpt-4o-mini",
                "provider_uuid": "oauth1",
                "origin": "inherit_main"
            },
            {
                "role": "safeguard",
                "effective_model": "",
                "configured_model": "legacy-model",
                "provider_uuid": "legacy"
            },
            { "role": "planner", "reason": "unassigned" }
        ]);
        let mut providers = HashMap::new();
        providers.insert("p1".into(), "Anthropic".into());
        let mut oauth = HashMap::new();
        oauth.insert("oauth1".into(), "Codex".into());
        let lines = role_usage_lines(&routes, &providers, &oauth);
        assert_eq!(
            lines[0],
            ("Main", "Anthropic · anthropic/claude-sonnet-4.5".into())
        );
        assert_eq!(lines[1], ("Awareness", "Codex · openai/gpt-4o-mini".into()));
        assert_eq!(lines[2], ("Safeguard", "legacy-model".into()));
        assert_eq!(lines[3], ("Compactor", "—".into()));
        assert_eq!(lines[4], ("Planner", "—".into()));
        assert_eq!(role_usage_lines(&Value::Null, &providers, &oauth)[0].1, "—");
    }

    #[test]
    fn provider_indexes_skip_blank_names() {
        let config = serde_json::json!({
            "providers": [
                {"id": "p", "name": " OpenAI "},
                {"id": "", "name": "x"},
                {"id": "q", "name": "  "}
            ]
        });
        let names = index_provider_names(config.get("providers"));
        assert_eq!(names.get("p").map(String::as_str), Some("OpenAI"));
        assert_eq!(names.len(), 1);
        let oauth = serde_json::json!([{"uuid": "o", "name": "Work"}]);
        let oauth_names = index_oauth_names(Some(&oauth));
        assert_eq!(oauth_names.get("o").map(String::as_str), Some("Work"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_proc_self_rss_is_nonzero() {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let rss = parse_vm_rss_bytes(&status).unwrap();
        assert!(rss > 0, "VmRSS missing: {status}");
        assert!(!proc_status_name(&status).is_empty());
        let info = std::fs::read_to_string("/proc/meminfo").unwrap();
        let total = parse_mem_total_bytes(&info).unwrap();
        assert!(total > rss);
    }
}
