//! Memory sample for the menu-bar card and the in-app usage popover.
//!
//! Token counters arrive on `Status` pushes. Resident memory is read here,
//! because the WebKit processes belong to this GUI, not to the session daemon.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::model::procmem::{
    apply_status, assemble_sample, index_oauth_names, index_provider_names, mem_mib_key,
    parse_pid_file, role_usage_lines, status_session_id, usage_live_json, AssembleInput,
    LiveTokens, MemSample,
};
#[cfg(target_os = "linux")]
use crate::model::procmem::{
    is_koma_or_webkit, parse_mem_total_bytes, parse_vm_rss_bytes, proc_status_name,
};

const SAMPLE_EVERY: Duration = Duration::from_secs(2);

pub(super) struct UsageTicker {
    session_id: String,
    tokens: LiveTokens,
    mem: MemSample,
    have_sample: bool,
    client_ready: bool,
    last_key: Option<(u64, u64, u64, u64)>,
    next_at: Instant,
    /// Snapshot `modelRoutes`. Null until the first snapshot.
    routes: serde_json::Value,
    seen_routes: bool,
    providers: HashMap<String, String>,
    oauth: HashMap<String, String>,
    roles: Vec<(&'static str, String)>,
}

impl UsageTicker {
    pub(super) fn new() -> Self {
        Self {
            session_id: String::new(),
            tokens: LiveTokens::default(),
            mem: MemSample::default(),
            have_sample: false,
            client_ready: false,
            last_key: None,
            next_at: Instant::now(),
            routes: serde_json::Value::Null,
            seen_routes: false,
            providers: HashMap::new(),
            oauth: HashMap::new(),
            roles: Vec::new(),
        }
    }

    /// Token counters, role routes, and provider names ride host pushes.
    /// Memory stays local to this process.
    pub(super) fn observe_push(&mut self, value: &serde_json::Value) {
        match value.get("k").and_then(|v| v.as_str()) {
            Some("Status") => {
                self.observe_status(value);
                self.publish_bar();
            }
            Some("Snapshot") => {
                let routes = value
                    .get("modelRoutes")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let changed = routes != self.routes || self.roles.is_empty();
                self.routes = routes;
                self.seen_routes = true;
                if changed {
                    self.rebuild_roles();
                }
            }
            Some("Config") => {
                self.providers = index_provider_names(value.get("providers"));
                self.rebuild_roles();
            }
            Some("OAuthState") => {
                self.oauth = index_oauth_names(value.get("conns"));
                self.rebuild_roles();
            }
            _ => {}
        }
    }

    pub(super) fn deadline(&self) -> Instant {
        self.next_at
    }

    /// Copy token fields off a host `Status` object. Other envelopes are ignored.
    pub(super) fn observe_status(&mut self, value: &serde_json::Value) {
        if !apply_status(&mut self.tokens, value) {
            return;
        }
        if let Some(id) = status_session_id(value) {
            if id != self.session_id {
                self.session_id = id.to_string();
            }
        }
    }

    /// The first host push means the page has installed `window.__komaClient`.
    /// A sample taken before that is returned once so the footer can show it.
    pub(super) fn note_host_push(&mut self) -> Option<String> {
        if self.client_ready {
            return None;
        }
        self.client_ready = true;
        self.take_live()
    }

    /// Sample RSS when `now` has reached the deadline. The menu bar updates on
    /// every sample. The web push is returned only when a 1 MiB bucket changed
    /// and the page can receive it.
    pub(super) fn poll(&mut self, now: Instant) -> Option<String> {
        if now < self.next_at {
            return None;
        }
        self.next_at = now + SAMPLE_EVERY;
        self.mem = sample_memory(&self.session_id);
        self.have_sample = true;
        self.publish_bar();
        if !self.client_ready {
            return None;
        }
        self.take_live()
    }

    fn rebuild_roles(&mut self) {
        if !self.seen_routes {
            return;
        }
        let next = role_usage_lines(&self.routes, &self.providers, &self.oauth);
        if next == self.roles {
            return;
        }
        self.roles = next;
        self.publish_bar();
    }

    pub(super) fn publish_bar(&self) {
        #[cfg(target_os = "macos")]
        {
            self.publish_macos();
            self.publish_roles();
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = self;
        }
    }

    fn take_live(&mut self) -> Option<String> {
        if !self.have_sample {
            return None;
        }
        let key = mem_mib_key(&self.mem);
        if self.last_key == Some(key) {
            return None;
        }
        self.last_key = Some(key);
        Some(usage_live_json(&self.mem))
    }

    #[cfg(target_os = "macos")]
    fn publish_macos(&self) {
        let stats = agent::computer_native::KomaUsageStats {
            mem_window: self.mem.window,
            mem_agent: self.mem.agent,
            mem_services: self.mem.services,
            mem_system: self.mem.system,
            tokens_in: self.tokens.tokens_in,
            tokens_cached: self.tokens.tokens_cached,
            tokens_out: self.tokens.tokens_out,
            cost_micros: self.tokens.cost_micros,
            context_window: self.tokens.context_window,
            working: u8::from(self.tokens.working),
        };
        // Safety: `stats` is a live `repr(C)` value. Callers run on the tao
        // main thread, after the window build has created NSApplication.
        unsafe { agent::computer_native::koma_usage_bar_update(&stats) };
    }

    #[cfg(target_os = "macos")]
    fn publish_roles(&self) {
        use std::os::raw::c_char;
        let labels: Vec<std::ffi::CString> = self
            .roles
            .iter()
            .map(|(label, _)| cstring_lossy(label))
            .collect();
        let values: Vec<std::ffi::CString> = self
            .roles
            .iter()
            .map(|(_, value)| cstring_lossy(value))
            .collect();
        let label_ptrs: Vec<*const c_char> = labels.iter().map(|text| text.as_ptr()).collect();
        let value_ptrs: Vec<*const c_char> = values.iter().map(|text| text.as_ptr()).collect();
        let count = label_ptrs.len() as u32;
        // Safety: the pointers stay valid for this call. The AppKit side copies
        // them into NSStrings before returning. A count of 0 clears the section.
        unsafe {
            agent::computer_native::koma_usage_bar_set_roles(
                if count == 0 {
                    std::ptr::null()
                } else {
                    label_ptrs.as_ptr()
                },
                if count == 0 {
                    std::ptr::null()
                } else {
                    value_ptrs.as_ptr()
                },
                count,
            );
        }
    }
}

#[cfg(target_os = "macos")]
fn cstring_lossy(text: &str) -> std::ffi::CString {
    let clean: String = text.chars().filter(|c| *c != '\0').take(200).collect();
    std::ffi::CString::new(clean).unwrap_or_default()
}

fn sample_memory(session_id: &str) -> MemSample {
    let self_pid = std::process::id();
    let helpers = helper_pids();
    assemble_sample(
        AssembleInput {
            self_pid,
            self_rss: resident(self_pid, false),
            system: system_bytes(),
            agent_pid: agent_pid(session_id),
            helper_pids: &helpers,
        },
        |pid| resident(pid, true),
        children_of,
    )
}

/// Pid files next to the sockets in [`crate::model::store`]: `mcp.pid`,
/// `oauth.pid`, and `linker.pid`. Advisory. A missing file contributes nothing.
fn helper_pids() -> Vec<u32> {
    let Ok(base) = crate::model::store::base_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for name in ["mcp.pid", "oauth.pid", "linker.pid"] {
        if let Some(pid) = read_pid(&base.join(name)) {
            out.push(pid);
        }
    }
    out
}

fn agent_pid(session_id: &str) -> Option<u32> {
    if session_id.is_empty() {
        return None;
    }
    let path = crate::model::store::daemon_pid_path(session_id).ok()?;
    read_pid(&path)
}

fn read_pid(path: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(path).ok()?;
    parse_pid_file(&text)
}

fn system_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        match std::fs::read_to_string("/proc/meminfo") {
            Ok(text) => parse_mem_total_bytes(&text).unwrap_or(0),
            Err(_) => 0,
        }
    }
    #[cfg(target_os = "macos")]
    {
        // Safety: no pointers. `hw.memsize` is a process-global sysctl.
        unsafe { agent::computer_native::koma_physical_memory() }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        0
    }
}

fn resident(pid: u32, require_ours: bool) -> u64 {
    #[cfg(target_os = "linux")]
    {
        linux_rss(pid, require_ours)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = require_ours;
        // Safety: `pid` is a plain integer. The name filter is inside the call.
        unsafe { agent::computer_native::koma_resident_size(pid) }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (pid, require_ours);
        0
    }
}

fn children_of(pid: u32) -> Vec<u32> {
    #[cfg(target_os = "linux")]
    {
        linux_children(pid)
    }
    #[cfg(target_os = "macos")]
    {
        macos_children(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn linux_rss(pid: u32, require_ours: bool) -> u64 {
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return 0;
    };
    if require_ours && !is_koma_or_webkit(proc_status_name(&status)) {
        return 0;
    }
    parse_vm_rss_bytes(&status).unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn linux_children(pid: u32) -> Vec<u32> {
    let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")) else {
        return Vec::new();
    };
    text.split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect()
}

#[cfg(target_os = "macos")]
fn macos_children(pid: u32) -> Vec<u32> {
    let mut buf = [0u32; 64];
    // Safety: `buf` is writable for 64 pids. The call writes at most that many.
    let n =
        unsafe { agent::computer_native::koma_child_pids(pid, buf.as_mut_ptr(), buf.len() as u32) };
    let n = (n as usize).min(buf.len());
    buf[..n].to_vec()
}
