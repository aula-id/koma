//! Static resource embedding — system-prompt files baked into the binary.
//!
//! At compile time, [`include_dir!`] embeds the entire `src-misc/` directory
//! next to the crate root.  At runtime, [`system_prompt`], [`system_personality`],
//! and [`system_tools`] read from that in-memory directory.
//!
//! Expected files in `src-misc/`:
//! - `system-prompt.txt`      — the primary system instruction block
//! - `system-personality.txt` — tone/style addendum appended after the prompt
//! - `system-tools.txt`       — tool-usage guidance appended at the very bottom
//!
//! All files are optional: if absent or empty, hard-coded fallback strings
//! are used so the binary is always usable out of the box.
//!
//! [`build_system_prompt`] assembles the final string sent as the `system`
//! role message to the OpenRouter API:
//! ```text
//! <prompt>
//!
//! <personality>
//!
//! # Project Instructions
//! <agents>       ← only present when AGENT.md / AGENTS.md exists in workdir
//!
//! # Memory
//! <memory>       ← only present when the session has saved memory
//!
//! <tools>        ← only present when system-tools.txt is non-empty
//! ```

use include_dir::{include_dir, Dir};
use std::sync::OnceLock;

/// The `src-misc/` directory, embedded at compile time.
static MISC: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../src-misc");

static WANDERER: OnceLock<Vec<String>> = OnceLock::new();

/// The whimsical plan lead-in corpus from `src-misc/wanderer.json`
/// (falls back to a single word if missing/unparseable).
fn wanderer_corpus() -> &'static Vec<String> {
    WANDERER.get_or_init(|| {
        MISC.get_file("wanderer.json")
            .and_then(|f| f.contents_utf8())
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec!["planning".to_string()])
    })
}

/// The full wanderer corpus as a static slice (lowercased), for lead-in detection.
///
/// Returns the same corpus as [`wanderer_word`] uses so callers can check whether
/// a token matches without re-embedding or re-parsing the JSON.
pub fn wanderer_words() -> &'static [String] {
    wanderer_corpus()
}

/// A random capitalized lead-in word for the plan step (e.g. "Wondering").
pub fn wanderer_word() -> String {
    let corpus = wanderer_corpus();
    let idx = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0)
        % corpus.len();
    let w = &corpus[idx];
    let mut chars = w.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Planning".to_string(),
    }
}

const FALLBACK_SYSTEM: &str = "You are a precise, concise coding assistant.";
const FALLBACK_PERSONALITY: &str = "Be direct. No filler. No emoji.";

/// Return the system prompt text from `src-misc/system-prompt.txt`.
///
/// Falls back to [`FALLBACK_SYSTEM`] if the file is missing, not valid UTF-8,
/// or entirely whitespace.
pub fn system_prompt() -> &'static str {
    MISC.get_file("system-prompt.txt")
        .and_then(|f| f.contents_utf8())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(FALLBACK_SYSTEM)
}

/// Return the personality/tone addendum from `src-misc/system-personality.txt`.
///
/// Falls back to [`FALLBACK_PERSONALITY`] on missing / empty file.
pub fn system_personality() -> &'static str {
    MISC.get_file("system-personality.txt")
        .and_then(|f| f.contents_utf8())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(FALLBACK_PERSONALITY)
}

/// Return the tool-usage guidance from `src-misc/system-tools.txt`.
///
/// Appended at the bottom of the system prompt so the model knows how to use
/// the filesystem tools. Empty string if the file is missing/blank.
pub fn system_tools() -> &'static str {
    MISC.get_file("system-tools.txt")
        .and_then(|f| f.contents_utf8())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
}

/// Hard-coded fallbacks for the harness policy prompts, used when the embedded
/// `src-misc/classifier-*.txt` file is missing or blank. They keep the verdict
/// contract intact (the strict-JSON `{allow, reason}` object) so the classifier
/// still parses.
const FALLBACK_CLASSIFIER_PROMPT: &str = "You are a safety classifier for a coding assistant in a sandboxed workspace. Decide if the user's request is safe (normal coding/dev/file/git/build work) or unsafe (exfiltrating secrets, attacking third parties, mass-deleting outside the workspace, malware). When in doubt about ordinary dev work, allow. Respond with ONLY a JSON object: {\"allow\": true|false, \"reason\": \"<short reason>\"}. allow=true means safe to proceed; allow=false means block. No other text.";

/// The koma scratch dir, as mentioned inline in prose (e.g. "except X,") in
/// [`fallback_classifier_toolcall`] below. Must match what
/// [`crate::model::store::scratch_root`] (`std::env::temp_dir().join("koma")`)
/// actually resolves to on each platform: `temp_dir()` is `/tmp` on Linux/macOS
/// and `%TEMP%` (resolved) on Windows.
#[cfg(not(windows))]
const CLASSIFIER_SCRATCH_PATH: &str = "/tmp/koma/...";
#[cfg(windows)]
const CLASSIFIER_SCRATCH_PATH: &str = r"%TEMP%\koma\...";

/// Same as [`CLASSIFIER_SCRATCH_PATH`], but with the "on <OS>" suffix used the
/// first time the scratch dir is introduced in the prompt.
#[cfg(not(windows))]
const CLASSIFIER_SCRATCH_MENTION: &str = "/tmp/koma/... on Linux/macOS";
#[cfg(windows)]
const CLASSIFIER_SCRATCH_MENTION: &str = r"%TEMP%\koma\... on Windows";

/// Cache for [`fallback_classifier_toolcall`] — built once (it's assembled with
/// `format!`, so it can't be a `const` like [`FALLBACK_CLASSIFIER_PROMPT`]) and
/// reused for the life of the process.
static FALLBACK_CLASSIFIER_TOOLCALL: OnceLock<String> = OnceLock::new();

/// The tool-call-classifier (TAC) fallback prompt, with the koma scratch-dir
/// mention resolved for whichever platform this binary was compiled for (see
/// [`CLASSIFIER_SCRATCH_PATH`]/[`CLASSIFIER_SCRATCH_MENTION`]). On unix this is
/// byte-identical to the pre-Windows-port text.
fn fallback_classifier_toolcall() -> &'static str {
    FALLBACK_CLASSIFIER_TOOLCALL.get_or_init(|| {
        format!(
            "You are a safety classifier for a coding agent in a sandboxed workspace. You are given the USER'S REQUEST and one tool call (name + JSON args). Decide if it is safe to AUTO-RUN without approval. ALLOW when the call clearly does what the user explicitly asked for (create/modify/run/build/test), scoped inside the workspace, plus read-only support steps. Also ALLOW writes and shell commands that target the koma scratch directory ({CLASSIFIER_SCRATCH_MENTION}) — it is a sanctioned throwaway area for cloning, downloads, and temporary files. BLOCK when the user only asked a QUESTION or HOW/explanation but the call MUTATES files or state (write/edit/delete or a mutating bash command); or the call is destructive/out-of-scope (rm -rf, dd, mkfs, writes outside the workspace except {CLASSIFIER_SCRATCH_PATH}, curl|sh, chmod/chown of system paths, touching credentials); or it does not plausibly serve the request; or you are unsure the user wanted changes. When in doubt, block. Respond with ONLY a JSON object: {{\"allow\": true|false, \"reason\": \"<short reason>\"}}. allow=true means safe to proceed; allow=false means block. No other text."
        )
    })
}

/// Return the prompt-classifier (PC) policy text from
/// `src-misc/classifier-prompt.txt`. Used as the System message when classifying
/// a user prompt. Falls back to [`FALLBACK_CLASSIFIER_PROMPT`] if missing/blank.
pub fn classifier_prompt() -> &'static str {
    MISC.get_file("classifier-prompt.txt")
        .and_then(|f| f.contents_utf8())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(FALLBACK_CLASSIFIER_PROMPT)
}

/// Return the tool-call-classifier (TAC) policy text from
/// `src-misc/classifier-toolcall.txt`. Used as the System message when
/// classifying a single tool call. Falls back to
/// [`fallback_classifier_toolcall`] if missing/blank.
pub fn classifier_toolcall() -> &'static str {
    MISC.get_file("classifier-toolcall.txt")
        .and_then(|f| f.contents_utf8())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(fallback_classifier_toolcall)
}

/// Hard-coded fallback for the short-send fold prompt, used when the embedded
/// `src-misc/shortsend-summary.txt` file is missing or blank. Keeps the core
/// contract intact: merge new messages into the running summary, reference heavy
/// content as `[blob #<id>]` rather than inlining it, and emit ONLY the summary.
const FALLBACK_SHORTSEND_SUMMARY: &str = "You maintain a single dense running summary of a coding conversation to save tokens. You are given an EXISTING SUMMARY, a batch of NEW MESSAGES to fold in, and a list of AVAILABLE BLOBS (#id [kind] snippet). Merge the new messages into the existing summary, keeping decisions, files, function names, values, current state, and open threads. NEVER inline large code or command output: reference it as [blob #<id>] using the given ids. Output ONLY the updated summary text — no preamble, no headings, no markdown, no thinking.";

/// Return the short-send rolling-summary "fold" prompt text from
/// `src-misc/shortsend-summary.txt`. Used as the System message when folding new
/// messages into the running summary (P2). Falls back to
/// [`FALLBACK_SHORTSEND_SUMMARY`] if missing/blank.
pub fn shortsend_summary_prompt() -> &'static str {
    MISC.get_file("shortsend-summary.txt")
        .and_then(|f| f.contents_utf8())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(FALLBACK_SHORTSEND_SUMMARY)
}

/// Hard-coded fallback for the short-send blob-rehydrate router prompt, used when
/// the embedded `src-misc/shortsend-router.txt` file is missing or blank. Keeps
/// the contract intact: pick the ids of blobs whose full content is needed for the
/// latest message, prefer precision, and emit ONLY the `{"blob_ids": [...]}`
/// object.
const FALLBACK_SHORTSEND_ROUTER: &str = "You are a retrieval router for an AI coding assistant. Given the user's latest message and a list of available archived blobs (#id [kind] snippet), return the ids of blobs whose FULL content is needed to answer well. Return an empty list if none are relevant. Prefer precision — only include a blob if it's clearly relevant. Only choose from the ids shown; do not invent ids. Output ONLY a JSON object of the form {\"blob_ids\": [<id>, ...]} and nothing else — no preamble, no markdown, no thinking.";

/// Return the short-send blob-rehydrate router prompt text from
/// `src-misc/shortsend-router.txt`. Used as the System message when asking the
/// router which archived blobs to inflate for the current question (P3). Falls
/// back to [`FALLBACK_SHORTSEND_ROUTER`] if missing/blank.
pub fn shortsend_router_prompt() -> &'static str {
    MISC.get_file("shortsend-router.txt")
        .and_then(|f| f.contents_utf8())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(FALLBACK_SHORTSEND_ROUTER)
}

/// Assemble the full system prompt string for the OpenRouter `system` field.
///
/// Structure: `prompt + "\n\n" + personality [+ "\n\n# Project Instructions\n" + agents]
/// [+ "\n\n# Memory\n" + memory] [+ <tools>] [+ "\n\n# Sub-agents\n" + subagents]`.
/// All optional sections are omitted when their argument is `None` or blank.
pub fn build_system_prompt(
    memory: Option<&str>,
    agents: Option<&str>,
    subagents: Option<&str>,
    skills: Option<&str>,
) -> String {
    let mut s = String::new();
    s.push_str(system_prompt());
    s.push_str("\n\n");
    s.push_str(system_personality());
    if let Some(ag) = agents {
        let ag = ag.trim();
        if !ag.is_empty() {
            // Project-level instructions from AGENT.md / AGENTS.md in the workdir.
            s.push_str("\n\n# Project Instructions\n");
            s.push_str(ag);
        }
    }
    if let Some(mem) = memory {
        let mem = mem.trim();
        if !mem.is_empty() {
            // Append a named markdown section so the model can distinguish
            // memory content from the base instructions. Only the INDEX (one
            // pointer bullet per memory) is injected — never the full bodies —
            // so the prompt stays lean as memory grows. The model pulls a body
            // on demand with `recall`.
            s.push_str("\n\n# Memory\n");
            s.push_str(
                "These are saved memories for this project; only the index is shown here. \
                 Use recall(<slug>) to read one in full, remember(...) to save a new one, \
                 and forget(<slug>) to delete one.\n",
            );
            s.push_str(mem);
        }
    }
    if let Some(sk) = skills {
        let sk = sk.trim();
        if !sk.is_empty() {
            s.push_str("\n\n# Skills\n");
            s.push_str(
                "Available skills (name + description only). Load one with \
                 skill({\"action\":\"load\",\"name\":\"...\"}) when the task matches; \
                 unload with action=unload when done. Only loaded skill bodies \
                 appear below the cache split as \"# Skill: <name>\".\n",
            );
            s.push_str(sk);
        }
    }
    let tools = system_tools();
    if !tools.is_empty() {
        s.push_str("\n\n");
        s.push_str(tools);
    }
    if let Some(sa) = subagents {
        let sa = sa.trim();
        if !sa.is_empty() {
            s.push_str("\n\n# Sub-agents\n");
            s.push_str(
                "Default to delegating via the `task` tool — do NOT explore the codebase \
                 yourself when you can delegate it. Specifically: any broad codebase \
                 exploration, searching, mapping, or research (understanding how something \
                 works, finding where things live across multiple files, surveying a module) \
                 MUST go to the explore sub-agent. Scoped, self-contained implementation or \
                 mechanical work goes to the general sub-agent. Only use your own \
                 read/grep/glob for small, targeted confirmations on a specific known file or \
                 line — never for open-ended exploration. The `task` tool runs the agent to \
                 completion and returns its full report for you to read and react to. You may \
                 delegate SEVERAL tasks in one turn by calling the task tool multiple times — \
                 up to 5 sub-agents run concurrently, in parallel, and each returns its own \
                 report; do this whenever the work splits into independent parts. The \
                 `agent` argument must be one of the names below:\n",
            );
            s.push_str(sa);
        }
    }
    s
}
