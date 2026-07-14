//! orchestrator-daemon: documentation-as-code for the grant-verbs that let
//! an extension DRIVE koma. It requires every grant that unlocks an
//! ext->koma `Call` verb except `models:contribute` (model/provider
//! registration is its own how-to — see `oauth-demo-daemon`'s comments) and
//! runs one scripted sequence through all five: `sessions.list`,
//! `models.invoke`, `context.set`, `chat.prompt`, `agents.spawn`.
//!
//! Read this file top to bottom like a manual, not like working code to
//! copy verbatim. Every call below is commented with its real request
//! shape, its real reply shape, and every error mode koma can hand back
//! (grant denial, payload caps, turn/queue budgets) — verified against
//! `src-agent/src/app/ext/broker.rs`. See `docs/EXTENSIONS.md`'s grants
//! reference table for the authoritative version of these limits; if the
//! two ever disagree, the doc (kept in lockstep with the broker) wins.
//!
//! # Demo mode note
//!
//! The SDK's canned `Koma` stub (used when `KOMA_EXT_SOCKET` is unset —
//! see `sdk.rs`'s `Koma::canned_result`) only fakes replies for `agents.*`
//! methods, because those were the first verbs the SDK shipped with. The
//! other four verbs below will print an honest
//! `{"error":"unknown method: <verb>"}` canned reply in demo mode; that is
//! expected, not a bug in this sample — we are not going to fake a nicer
//! answer just to make the demo transcript look complete. Run this against
//! a real koma (`KOMA_EXT_SOCKET` set) to see the real replies documented
//! in the comments below.

use koma_extension::{run_daemon, DaemonDemo, Extension, ExtensionManifest, Koma};

struct Orchestrator;

impl Extension for Orchestrator {
    fn manifest(&self) -> ExtensionManifest {
        serde_json::from_str(include_str!("../manifest.json")).expect("manifest.json is valid")
    }
}

fn drive(koma: &mut Koma) {
    // --- 1. sessions.list (requires `sessions:manage`) ----------------------
    // Params: {} — no arguments.
    // Reply:  [ { "id": <uuid>, "name": <string|null>, "workdir": <string>,
    //             "live": <bool>, "working": <bool> }, ... ]
    // This is a registry snapshot merged with a live-daemon probe sweep, not
    // a subscription — call it again whenever you need a fresh view. v1
    // limit: no cross-daemon polling. It can tell you a session IS live; it
    // cannot stream that session's state to you.
    let sessions = koma.call("sessions.list", serde_json::json!({}));
    println!("1. sessions.list -> {sessions}");

    // --- 2. models.invoke (requires `models:invoke`) -------------------------
    // Params: { "prompt": <string, required, <=32KB>, "role"?: "main" |
    //           "awareness" | "safeguard" | "compactor" | "planner"
    //           (default "main"), "system"?: <string> }
    // Reply:  { "output": <string>, "model": <model id string> }
    // Errors: empty prompt -> "models.invoke requires a non-empty 'prompt'";
    //         over 32KB -> "prompt exceeds 32KB"; unrecognized role ->
    //         "unknown role" (never silently falls back to Main); no
    //         dispatchable/authed route for the role -> "no usable route
    //         for role <role>" / "role <role> route is not dispatchable
    //         (Anthropic-compatible not wired)" / "role <role> route has no
    //         usable auth"; a stuck backend -> "model call timed out" after
    //         koma's internal 25s budget (deliberately shorter than the 30s
    //         broker call ceiling, so you always get a value back instead
    //         of a raw transport timeout).
    let classification = koma.call(
        "models.invoke",
        serde_json::json!({
            "role": "main",
            "prompt": "classify as bug-report or feature-request:\n\n\"the export button does nothing on Safari\""
        }),
    );
    println!("2. models.invoke -> {classification}");

    // --- 3. context.set (requires `context:publish`) -------------------------
    // Params: { "text": <string, <=8KB per extension> }
    // Reply:  { "ok": true }
    // Errors: over 8KB -> "context exceeds 8KB". An empty/whitespace `text`
    // is not an error — it CLEARS your entry (still replies { "ok": true }).
    // Your text rides the system prompt's volatile tail on every turn,
    // placed after the prompt-cache split, so publishing here never busts
    // the cached prompt head.
    let ctx = koma.call(
        "context.set",
        serde_json::json!({ "text": "orchestrator-daemon: 1 session watched; last classification: bug-report" }),
    );
    println!("3. context.set -> {ctx}");

    // --- 4. chat.prompt (requires `chat:prompt`) ------------------------------
    // Params: { "text": <string, required, <=16KB> }
    // Reply:  { "queued": <new queue length> }
    // This does NOT inject into the chat immediately: it buffers onto the
    // active session, and koma injects it as one synthetic user turn the
    // next time that session goes idle, so it can never corrupt an
    // in-flight turn's tool-call/tool-result ordering. Errors: empty text ->
    // "chat.prompt requires a non-empty 'text'"; over 16KB -> "prompt
    // exceeds 16KB"; a 6th concurrent entry -> "prompt queue full (5)" (cap
    // 5; an exact repeat of the last queued entry is silently deduped
    // instead of erroring or double-counting); and once this extension has
    // injected 10 turns without the user typing anything themselves ->
    // "extension turn budget exhausted; waiting for user activity" (the
    // budget resets on real user input — it exists so a runaway extension
    // can't talk to itself forever).
    let prompt = koma.call(
        "chat.prompt",
        serde_json::json!({
            "text": "a new bug report came in, classified as: export button does nothing on Safari. want me to spawn an investigation agent?"
        }),
    );
    println!("4. chat.prompt -> {prompt}");

    // --- 5. agents.spawn (requires `agents:orchestrate`) ----------------------
    // Params: { "task": <string, required non-empty>, "agent"?: <string,
    //           default "general">, "model"?: <string slug>, "effort"?:
    //           <string>, "notify"?: <bool, default false> }
    // Reply:  { "agentId": <u64, ext-facing>, "status": "spawned" } — or
    //         { "agentId": <u64>, "status": "queued" } once 5 sub-agents are
    //         already running (koma's MAX_SUBAGENTS cap; this one still
    //         gets an id and starts once a slot frees up).
    // `notify: true` means koma will ALSO fire a private "agents.done" event
    // straight to THIS extension when the agent reaches a terminal state —
    // independent of `contributes.events` (see `event-watcher-daemon` for
    // the subscription-gated broadcast events, which this is not one of).
    // Errors: empty task -> "agents.spawn requires a non-empty 'task'"; no
    // foreground session -> "no active session"; unknown agent or no
    // client -> "failed to spawn agent '<agent>' (no client/session or
    // unknown agent)".
    let spawned = koma.call(
        "agents.spawn",
        serde_json::json!({
            "task": "investigate: export button does nothing on Safari",
            "model": "main",
            "effort": "medium",
            "notify": true
        }),
    );
    println!("5. agents.spawn -> {spawned}");
}

fn main() {
    run_daemon(
        Orchestrator,
        DaemonDemo {
            invoke: None,
            driver: Some(drive),
        },
    );
}
