//! App-wide compile-time constants.
//!
//! These values are used across the codebase for defaults and identity strings.
//! None of them are user-configurable at runtime — they represent sensible
//! starting points that the user can override via the credentials form.

/// Base URL of the OpenRouter API endpoint.
pub const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Model identifier sent to OpenRouter when the user hasn't specified one.
pub const DEFAULT_MODEL: &str = "openai/gpt-4o-mini";

/// Default OpenRouter provider slug (empty = use OpenRouter's default routing).
///
/// An empty string means no provider pin is applied; the `provider` field is
/// omitted from the request body so OpenRouter selects the route automatically.
pub const DEFAULT_PROVIDER: &str = "";

/// How many most-recent messages to keep when compacting the conversation.
///
/// `/compact` preserves the system prompt plus the last `DEFAULT_PRESERVE_N`
/// turns so the model retains recent context while the token count shrinks.
pub const DEFAULT_PRESERVE_N: usize = 6;

/// Default secondary model used to summarise the project's docs for the
/// self-awareness block. A small, cheap model: the summary is short and
/// regenerated each session, so capability beyond "read docs, write 4-6
/// sentences" is wasted spend.
pub const DEFAULT_AWARENESS_MODEL: &str = "openai/gpt-oss-20b";

/// Default provider slug for the awareness summary call (strict-pinned). Groq
/// serves the default awareness model fast and cheap.
pub const DEFAULT_AWARENESS_PROVIDER: &str = "groq";

/// Default safety-classifier model (the harness "Pass B"). A dedicated
/// safeguard model that judges whether a user prompt / tool call is safe to
/// proceed. Routed via OpenRouter (see `DEFAULT_CLASSIFIER_PROVIDER`).
pub const DEFAULT_CLASSIFIER_MODEL: &str = "openai/gpt-oss-safeguard-20b";

/// Default provider slug for the safety-classifier call (strict-pinned). Groq
/// serves the safeguard model fast and cheap, which keeps the per-call latency
/// low enough to gate tool execution synchronously.
pub const DEFAULT_CLASSIFIER_PROVIDER: &str = "groq";

/// Value sent as the `HTTP-Referer` header with every OpenRouter request.
///
/// OpenRouter uses this to attribute usage to the originating project.
pub const HTTP_REFERER: &str = "https://koma.run";

/// Human-readable application name (displayed in the TUI title bar).
pub const APP_TITLE: &str = "koma";

/// Name of the hidden directory created in the user's home folder to store
/// session files and configuration.
pub const APP_DIR_NAME: &str = ".koma";

/// Hard cap on a single tool result's size, in characters. ~100k tokens at
/// ~4 chars/token. Tool outputs are not truncated below this.
pub const MAX_TOOL_OUTPUT_CHARS: usize = 400_000;

/// Hard ceiling on a sub-agent's final report before it is delivered to the
/// main agent as a `task` tool result. Reports above this are truncated (with a
/// marker) so several sub-agents can't overflow the main model's context window.
/// ~12.5k tokens at ~4 chars/token. The sub-agent prompt also asks for concise
/// reports (see subagent::context) — this is the safety net.
pub const MAX_SUBAGENT_REPORT_CHARS: usize = 50_000;
