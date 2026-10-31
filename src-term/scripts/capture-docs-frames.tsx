/**
 * Render the real terminal components and write truecolor frames for the docs.
 * Usual window: 80 columns by 38 rows. The too-narrow screen stays under 50 columns.
 * The too-narrow screen is the one capture under 50 columns.
 *
 *   bun scripts/capture-docs-frames.tsx
 */
import { mkdirSync, writeFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { spyOn } from "bun:test"
import { act, useMemo, type ReactNode } from "react"
import { SyntaxStyle, TextAttributes, getBaseAttributes, type RGBA } from "@opentui/core"
import { useRenderer } from "@opentui/react"
import { testRender } from "@opentui/react/test-utils"
import { Chat } from "../src/chat.tsx"
import { Hub } from "../src/hub.tsx"
import { OpeningSplash, WarmingSplash } from "../src/loading.tsx"
import { placeOf } from "../src/modes/place.ts"
import { ModeSurface } from "../src/modes/surface.tsx"
import * as paths from "../src/paths.ts"
import { QuitScreen } from "../src/quit.tsx"
import type { ChatMessage, GlobalSnap, SessionSnap, Snapshot } from "../src/protocol.ts"
import { themeFor, type Theme } from "../src/theme.ts"

const COLS = 80
const ROWS = 38
const outDir = resolve(dirname(fileURLToPath(import.meta.url)), "../../src-docs/src/captured")

type Frame = { cols: number; rows: number; ansi: string }

function channel(color: RGBA): [number, number, number] {
  const [r, g, b] = color.toInts()
  const scale = (value: number) => (value > 255 ? Math.round(value / 257) : value)
  return [scale(r), scale(g), scale(b)]
}

function toAnsi(view: Awaited<ReturnType<typeof testRender>>, cols: number, rows: number): string {
  const lines = view.captureSpans().lines.slice(0, rows).map((line) => {
    let text = ""
    let used = 0
    for (const span of line.spans) {
      if (used >= cols) break
      const chunk = [...span.text].slice(0, cols - used).join("")
      used += [...chunk].length
      const attr = getBaseAttributes(span.attributes)
      const open: string[] = []
      if (attr & TextAttributes.BOLD) open.push("1")
      if (attr & TextAttributes.DIM) open.push("2")
      if (attr & TextAttributes.ITALIC) open.push("3")
      if (attr & TextAttributes.UNDERLINE) open.push("4")
      if (attr & TextAttributes.INVERSE) open.push("7")
      if (attr & TextAttributes.STRIKETHROUGH) open.push("9")
      const [fr, fg, fb] = channel(span.fg)
      const [br, bg, bb] = channel(span.bg)
      open.push(`38;2;${fr};${fg};${fb}`, `48;2;${br};${bg};${bb}`)
      text += `\x1b[${open.join(";")}m${chunk}\x1b[0m`
    }
    return text
  })
  while (lines.length < rows) lines.push("")
  return lines.join("\n")
}

function Shell({ children }: { children: ReactNode }) {
  const theme = themeFor("dark")
  return (
    <box flexGrow={1} flexDirection="column" backgroundColor={theme.bg} paddingLeft={2} paddingRight={2}>
      {children}
    </box>
  )
}

function Styled({ children }: { children: (style: SyntaxStyle) => ReactNode }) {
  const renderer = useRenderer()
  const style = useMemo(() => SyntaxStyle.fromStyles({}, renderer.nativeScene), [renderer])
  return children(style)
}

function session(patch: Partial<SessionSnap> = {}): SessionSnap {
  return {
    id: "launch",
    name: "launch notes",
    cwd: "/Users/ada/notes",
    messages: [],
    streaming: null,
    stream_reasoning: "",
    working: false,
    waiting: false,
    awaiting_approval: false,
    approval_reason: null,
    pending_tool_calls: [],
    tool_idx: 0,
    resolved_model_id: "gpt-4.1",
    effort: "",
    pending_steer: [],
    pending_steer_attachments: [],
    subagents: [],
    pending_subagents: [],
    plan_todos: [],
    tokens_in: 0,
    tokens_out: 0,
    tokens_cached: 0,
    cost: 0,
    context_usage: null,
    sdlc_phase: null,
    sdlc_branch: null,
    ...patch,
  }
}

function globalSnap(patch: Partial<GlobalSnap> = {}): GlobalSnap {
  return {
    input: "",
    cursor: 0,
    status: "ready",
    palette: "dark",
    accent: "green",
    mode: "Chat",
    toast: null,
    toast_event_id: null,
    file_palette: null,
    command_palette: [],
    palette_sel: 0,
    agent_mode: "normal",
    pending_attachments: [],
    work_elapsed_ms: null,
    follow: true,
    latest_version: null,
    current_version: "0.4.6",
    pending_steer_sel: 0,
    pending_steer_focus: false,
    subagents_open: false,
    subagent_sel: 0,
    agent_viewer: null,
    ...patch,
  }
}

function snap(sessionPatch: Partial<SessionSnap> = {}, globalPatch: Partial<GlobalSnap> = {}): Snapshot {
  return { foreground_id: "launch", sessions: [session(sessionPatch)], global: globalSnap(globalPatch) }
}

const user: ChatMessage = { role: "user", content: "Summarize the notes in this folder." }
const reply: ChatMessage = { role: "assistant", content: "I read three notes. The launch date is 4 June, and the open question is the venue." }

function chat(sessionPatch: Partial<SessionSnap> = {}, globalPatch: Partial<GlobalSnap> = {}, extra?: { rewind?: { entries: string[]; selected: number }; overlay?: ReactNode; modal?: ReactNode }) {
  const theme = themeFor("dark")
  return (
    <Styled>
      {(syntaxStyle) => (
        <Chat
          snap={snap(sessionPatch, globalPatch)}
          theme={theme}
          syntaxStyle={syntaxStyle}
          focused={!extra?.modal}
          rewind={extra?.rewind ?? null}
          overlay={extra?.overlay ?? null}
          modalOverlay={extra?.modal ?? null}
          send={() => {}}
          sendTracked={async () => "ack"}
          onFlushError={() => {}}
        />
      )}
    </Styled>
  )
}

function mode(name: string, body: Record<string, unknown>, columns = COLS) {
  return <ModeSurface mode={{ [name]: body }} theme={themeFor("dark")} columns={columns} onMove={() => {}} send={() => {}} />
}

/** Full screens replace the chat. Overlays sit above the composer. */
function surface(name: string, body: Record<string, unknown>) {
  const screen = mode(name, body)
  if (placeOf({ [name]: body }) === "full") return screen
  return chat({ messages: [user, reply] }, {}, { overlay: screen })
}

function assertFrame(name: string, text: string) {
  if (text.includes("▍")) throw new Error(`${name} captured a tip still typing`)
  let open = 0
  for (const line of text.split("\n")) {
    const opens = line.split("╭").length - 1
    // A box drawn inside another one keeps the outer side borders on that line.
    // The broken agents frame starts the composer with no outer border left.
    if (opens > 0 && open > 0 && !line.includes("│")) throw new Error(`${name} opens a panel before the previous one closes`)
    open += opens - (line.split("╰").length - 1)
    if (open < 0) open = 0
  }
}

const commands = [
  { name: "/new", desc: "Spawn a new session, swap to it (current keeps running)" },
  { name: "/new kill", desc: "Spawn a new session and close the current one" },
  { name: "/new remote", desc: "Start a new session on a saved remote host" },
  { name: "/resume", desc: "Open the session hub (live + past sessions)" },
  { name: "/resume remote", desc: "Resume a session on a saved remote host" },
  { name: "/mode", desc: "Cycle or explicitly set the agent mode" },
  { name: "/settings", desc: "Edit key, model, provider, theme, name" },
  { name: "/help", desc: "List the available commands" },
]

async function settle(view: Awaited<ReturnType<typeof testRender>>) {
  await act(async () => { await view.waitForVisualIdle() })
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 250))
    await view.waitForVisualIdle()
  })
}

async function readFrame(view: Awaited<ReturnType<typeof testRender>>): Promise<string> {
  for (let attempt = 0; attempt < 12; attempt++) {
    try {
      return view.captureCharFrame()
    } catch (error) {
      if (!String(error).includes("OutputBusy") || attempt === 11) throw error
      await act(async () => { await new Promise((resolve) => setTimeout(resolve, 40)) })
    }
  }
  return ""
}

async function shoot(name: string, node: ReactNode, cols = COLS, rows = ROWS, then?: (view: Awaited<ReturnType<typeof testRender>>) => void | Promise<void>): Promise<Frame> {
  const view = await testRender(<Shell>{node}</Shell>, { width: cols, height: rows })
  try {
    await settle(view)
    if (then) {
      await act(async () => { await then(view) })
      await settle(view)
    }
    const started = Date.now()
    let visible = await readFrame(view)
    while (visible.includes("▍") && Date.now() - started < 3000) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 40))
        await view.waitForVisualIdle()
      })
      visible = await readFrame(view)
    }
    if (!visible.trim()) throw new Error(`${name} captured an empty frame`)
    assertFrame(name, visible)
    const ansi = toAnsi(view, cols, rows)
    return { cols, rows, ansi }
  } finally {
    await act(async () => { await view.dispose() })
  }
}

async function main() {
  spyOn(paths, "listSessions").mockResolvedValue({
    live: [
      { session_id: "a1b2c3d4e5f6", name: "launch notes", pwd: "/Users/ada/notes", working: false },
    ],
    history: [
      { session_id: "c9d8e7f6a5b4", name: "quarterly brief", pwd: "/Users/ada/notes", dir_label: "notes", updated_at: Math.floor(Date.now() / 1000) - 86400 * 2 },
    ],
  })
  spyOn(paths, "killSession").mockResolvedValue(undefined)
  spyOn(paths, "deleteSession").mockResolvedValue(undefined)

  const theme = themeFor("dark")
  const frames: Record<string, Frame> = {}
  const save = async (name: string, node: ReactNode, cols = COLS, rows = ROWS, then?: (view: Awaited<ReturnType<typeof testRender>>) => void | Promise<void>) => {
    frames[name] = await shoot(name, node, cols, rows, then)
    process.stdout.write(`captured ${name} ${cols}x${rows}\n`)
  }

  await save("welcome-0", mode("Onboard", { cursor: 0 }))
  await save("welcome-1", mode("Onboard", { cursor: 1 }))
  await save("welcome-2", mode("Onboard", { cursor: 2 }))
  await save("sign-in", mode("OnboardProvider", { step: "pick", oauth_flow: { kind: "pick", cursor: 0, options: ["OpenAI", "Anthropic", "koma.run"] } }))
  await save("own-key", mode("KeyInput", { first_run: true, step: 0, field: 0, editing: true, endpoint: "https://api.example.com/v1", api_key: "sk-secret-ab" }))
  await save("own-key-model", mode("KeyInput", { first_run: true, step: 1, query: "gpt-4.1", results: ["gpt-4.1", "gpt-4.1-mini"], result_sel: 0 }))
  await save("opening", <OpeningSplash theme={theme} />)
  await save("warming", <WarmingSplash theme={theme} warm={{ elapsedMs: 1200, workspace: { phase: "running", detail: "" }, awareness: { phase: "pending", detail: "" } }} />)
  await save("chat-ready", chat())
  await save("chat-draft", chat({}, { input: "Summarize the notes in this folder.", cursor: 36 }))
  await save("chat-reply", chat({ messages: [user, reply] }))
  await save("chat-you", chat({ messages: [user] }))
  await save("chat-working", chat({ messages: [user], working: true, stream_reasoning: "Reading the notes…" }, { status: "thinking", work_elapsed_ms: 4200 }))
  await save("chat-stopped", chat({ messages: [user, { role: "assistant", content: "Stopped. Tell me what to change." }] }, { status: "interrupted" }))
  await save("chat-multiline", chat({}, { input: "Draft a reply to the client.", cursor: 28, status: "ready" }))
  await save("chat-file", chat({}, { input: "@brief", cursor: 6, file_palette: ["brief.md", "brief-notes.md"], palette_sel: 0 }))
  await save("palette-new", chat({}, { input: "/new", cursor: 4, command_palette: commands, palette_sel: 0 }))
  await save("palette-new-kill", chat({}, { input: "/new kill", cursor: 9, command_palette: commands, palette_sel: 1 }))
  await save("palette-new-remote", chat({}, { input: "/new remote", cursor: 11, command_palette: commands, palette_sel: 2 }))
  await save("palette-resume", chat({}, { input: "/resume", cursor: 7, command_palette: commands, palette_sel: 3 }))
  await save("palette-resume-remote", chat({}, { input: "/resume remote", cursor: 14, command_palette: commands, palette_sel: 4 }))
  await save("chat-mode-auto", chat({}, { agent_mode: "auto" }))
  await save("chat-mode-normal", chat({}, { agent_mode: "normal" }))
  await save("chat-mode-plan", chat({}, { agent_mode: "plan" }))
  await save("chat-mode-sdlc", chat({ sdlc_phase: "assess" }, { agent_mode: "sdlc" }))
  await save("chat-mode-yolo", chat({}, { agent_mode: "yolo" }))
  await save("chat-free", chat({ resolved_model_id: "koma-free" }, { input: "/free", cursor: 5, agent_mode: "auto", command_palette: [{ name: "/free", desc: "Toggle this session to use koma-free" }] }))
  await save("chat-update", chat({ messages: [user, reply] }, { input: "/update", cursor: 7, latest_version: "0.4.7", command_palette: [{ name: "/update", desc: "Install the latest koma and reopen this session" }] }))
  await save("chat-rename", chat({ name: "quarterly brief" }, { input: "/rename quarterly brief", cursor: 23, command_palette: [{ name: "/rename", desc: "Rename the current session" }] }))
  await save("chat-cd", chat({}, { input: "/cd ~/Clients/acme", cursor: 18, command_palette: [{ name: "/cd", desc: "Change the session working directory" }] }))
  await save("chat-adddir", chat({}, { input: "/adddir ~/Clients/acme/assets", cursor: 30, command_palette: [{ name: "/adddir", desc: "Add another folder" }] }))
  await save("chat-compact", chat({}, { input: "/compact", cursor: 8, command_palette: [{ name: "/compact", desc: "Summarize the conversation" }] }))
  await save("chat-clear", chat({}, { input: "/clear", cursor: 6, command_palette: [{ name: "/clear", desc: "Clear the chat and keep the archive" }] }))
  await save("chat-select", chat({}, { input: "/select", cursor: 7, command_palette: [{ name: "/select", desc: "Copy the conversation" }] }))
  await save("chat-task", chat({}, { input: "/task editor Rewrite the brief", cursor: 30, command_palette: [{ name: "/task", desc: "Hand a job to an assistant" }] }))
  await save("rewind", chat({}, {}, { rewind: { entries: ["Summarize the notes in this folder.", "Draft a reply to the client."], selected: 1 } }))

  const quit = (selected: 0 | 1 | 2) => chat({}, {}, { modal: <QuitScreen state={{ working: 1, total: 1, selected }} theme={theme} exiting={null} onPick={() => {}} /> })
  await save("quit-0", quit(0))
  await save("quit-1", quit(1))
  await save("quit-2", quit(2))

  await save("hub-cooking", <Hub theme={theme} onOpen={() => {}} />)
  await save("hub-history", <Hub theme={theme} onOpen={() => {}} />, COLS, ROWS, (view) => { view.mockInput.pressTab() })

  const settings = (page: string, extra: Record<string, unknown> = {}) => mode("Settings", {
    page,
    palette: "dark",
    palette_sel: 0,
    name: "launch notes",
    workdir: ["/Users/ada/notes"],
    awareness_enabled: true,
    classifier_enabled: true,
    allowed_folders: [],
    short_send_enabled: true,
    bash_saving: true,
    coding_autosave: false,
    internet_mode: "simple",
    web_search: { provider: "built_in" },
    mouse_capture: "auto",
    subagent_max_turns: 8,
    max_output_tokens: 0,
    providers: [
      { name: "OpenAI", endpoint: "https://api.openai.com/v1" },
      { name: "Your endpoint", endpoint: "https://api.example.com/v1" },
    ],
    prov_sel: 0,
    oauth_drafts: [
      { label: "ChatGPT", provider: "openai", status: "connected" },
      { label: "Claude", provider: "anthropic", status: "not connected" },
    ],
    oauth_sel: 0,
    oauth_flow: { kind: "idle" },
    models: [
      { name: "gpt-4.1", model_id: "gpt-4.1", roles: ["main"], provider_idx: 0 },
      { name: "planner", model_id: "gpt-4.1-mini", roles: ["planner"], provider_idx: 2 },
    ],
    model_sel: 5,
    ...extra,
  })
  await save("settings-quick", settings("quick"))
  await save("settings-appearance", settings("appearance"))
  await save("settings-general", settings("general"))
  await save("settings-providers", settings("providers"))
  await save("settings-oauth", settings("oauth"))
  await save("settings-models", settings("models"))
  await save("settings-internet-full", settings("general", { internet_mode: "full" }))

  await save("effort", surface("Effort", { options: ["low", "medium", "high"], selected: 1, note: "medium is the usual effort" }))
  await save("model", surface("Model", { sub: "roles", options: [["main", "main"], ["awareness", "awareness"], ["safeguard", "safeguard"], ["compactor", "compactor"], ["planner", "planner"]], cursor: 0, lines: [] }))
  await save("todo", surface("Todo", { selected: 0, items: [
    { content: "Confirm the new date with the client", status: "open", priority: "" },
    { content: "Update brief.md", status: "completed", priority: "" },
    { content: "Send the revised deck", status: "open", priority: "" },
  ] }))
  await save("bash", surface("Bash", { selected: 0, jobs: [
    { command: "npm test", status: "running", running: true, elapsed_secs: 12, output_tail: "" },
    { command: "git status", status: "done", running: false, elapsed_secs: 1, output_tail: "" },
  ] }))
  await save("bash-bg", surface("Bash", { selected: 0, jobs: [{ command: "npm test", status: "background", running: true, elapsed_secs: 40, output_tail: "" }] }))
  await save("skill", surface("Skill", { chip: "all", selected: 0, all: [
    { name: "client-brief", description: "How the team writes a client brief", is_active: true },
    { name: "weekly-update", description: "The Monday note", is_active: false },
  ] }))
  await save("help", surface("Help", { query: "", selected: 0, all: [
    { key: "/settings", desc: "Edit key, model, provider, theme, name", kind: "command" },
    { key: "/resume", desc: "Open the session hub", kind: "command" },
    { key: "Enter", desc: "send message / run command", kind: "keybinding" },
  ] }))
  await save("notifications", surface("Notifications", { query: "", cursor: 0, app: false, entries: [
    { id: "1", message: "Saved the brief", source: "koma", severity: "info", timestamp: Date.now(), read: false },
    { id: "2", message: "Could not reach the account", source: "provider", severity: "error", timestamp: Date.now() - 60_000, read: true },
  ] }))
  await save("usage", surface("Usage", { view: "session", range: "today" }))
  await save("agents", surface("Agents", { mode: "browse", list_sel: 0, agents: [
    { name: "researcher", description: "looks things up", source: "global" },
    { name: "editor", description: "rewrites for a client", source: "session" },
  ] }))
  await save("mcp", surface("Mcp", { mode: "browse", list_sel: 0, servers: [
    { name: "company wiki", command: "", url: "https://wiki.example", transport: "http" },
    { name: "issue tracker", command: "", url: "", transport: "stdio" },
  ] }))
  await save("extensions", surface("Extensions", { mode: "browse", list_sel: 0, rows: [
    { name: "calendar", version: "1.2.0", running: false, active: true },
    { name: "crm", version: "0.9.0", running: false, active: false },
  ] }))
  await save("extension-use", surface("Extensions", { mode: "use", list_sel: 0, rows: [
    { name: "calendar", version: "1.2.0", active: false },
    { name: "crm", version: "0.9.0", active: true },
  ] }))
  await save("store", surface("ExtStore", { mode: "browse", list_sel: 0, rows: [
    { name: "calendar", latest_version: "1.2.0", installed: false, tier: "free" },
    { name: "crm", latest_version: "0.9.0", installed: true, tier: "free" },
  ] }))
  await save("remote", surface("Remote", { sub: "browse", selected: 0, hosts: [
    { name: "office-mac", user: "ada", host: "office.local", port: "22", connected: true },
    { name: "build-server", user: "ada", host: "build.local", port: "22", connected: false },
  ] }))
  await save("security", surface("Security", { selected: 0, yolo_armed: false, status: { running: true, installed: true, tools: [{ name: "review", domain: "repo" }] }, inactive: [] }))

  await save("too-narrow", (
    <box flexGrow={1} flexDirection="column" justifyContent="center" alignItems="center" backgroundColor={theme.bg} gap={1}>
      <text fg={theme.error}>Terminal too small</text>
      <text fg={theme.fg}>42 columns</text>
      <text fg={theme.dim}>need 50</text>
      <text fg={theme.fg}>Resize if you can</text>
      <text fg={theme.dim}>q  esc  ctrl+c</text>
    </box>
  ), 42, 24)

  mkdirSync(outDir, { recursive: true })
  for (const [name, frame] of Object.entries(frames)) {
    writeFileSync(resolve(outDir, `${name}.json`), JSON.stringify(frame))
  }
  writeFileSync(resolve(outDir, "index.ts"), `/* Generated by src-term/scripts/capture-docs-frames.tsx. Do not edit. */\nexport type CapturedFrame = { cols: number; rows: number; ansi: string }\n${Object.keys(frames).map((name) => {
    const id = name.replace(/-([a-z0-9])/g, (_, c: string) => c.toUpperCase())
    return `import ${id}Data from "./${name}.json"\nexport const ${id} = ${id}Data as CapturedFrame`
  }).join("\n")}\n`)
  process.stdout.write(`wrote ${Object.keys(frames).length} frames\n`)
  process.exit(0)
}

await main()
