import type { WireKey as KeyWire } from "../keys.ts"

export type MenuAction =
  | { Select: { group: string; index: number; id?: string | null } }
  | { Replace: { field: string; value: string } }
  | { Key: KeyWire }
  | "Save" | "Discard" | "KeepEditing"
export type MenuRequest = { Menu: { request_id: string; mode: string; action: MenuAction } }
export type ActionDescriptor = { label: string; key?: KeyWire; action?: MenuAction; primary?: boolean; disabled?: boolean }
export type Send = (request: unknown) => void

export function select(send: Send, index: number, group = "active", id?: string) {
  send({ MenuSelect: { group, index, id } })
}
export function replaceField(send: Send, field: string, value: string) {
  send({ MenuReplace: { field, value } })
}
export function key(code: KeyWire["code"], mods = 0): KeyWire { return { code, mods } }
export function button(label: string, code: KeyWire["code"], mods = 0): ActionDescriptor { return { label, key: key(code, mods) } }
export const back = button("Back (esc)", "Esc")
const enter = (label: string) => button(`${label} (enter)`, "Enter")
const char = (label: string, c: string) => button(`${label} (${c})`, { Char: c })

/** Explicit controls per domain state. Labels are never interpreted as commands. */
export function menuControls(name: string, b: Record<string, unknown>): ActionDescriptor[] {
  const mode = b.mode
  if (name === "Settings") {
    if (b.search_editor) return [(b.search_editor as {confirmed?:boolean}).confirmed ? enter("Save") : enter("Choose provider"), button((b.search_editor as {confirmed?:boolean}).confirmed ? "Return to providers (esc)" : "Cancel (esc)", "Esc")]
    if (b.model_provider_cursor != null) return [enter("Choose provider"), button("Cancel chooser (esc)", "Esc")]
    if (b.model_editing || b.provider_editing) return [enter("Confirm field"), button("Browse fields (esc)", "Esc")]
    if (b.page === "model_form" || b.page === "provider_form") return [enter("Edit / choose"), button("Field (tab)", "Tab"), back]
    const flow = b.oauth_flow as {kind?: string} | undefined
    if (b.page === "oauth" && flow?.kind && flow.kind !== "idle") return oauthControls(flow.kind)
    if (b.page === "appearance") return [enter("Apply"), back]
    return [back]
  }
  if (name === "KeyInput" && b.step === 0) return [enter(b.editing ? "Continue" : "Edit field"), button("Field (tab)", "Tab"), back]
  if (name === "OnboardProvider" && b.step !== "model_select") return oauthControls((b.oauth_flow as {kind?:string})?.kind ?? "pick")
  if (mode === "delete_confirm" || mode === "uninstall_confirm") return [char(mode === "delete_confirm" ? "Delete" : "Uninstall", "y"), button("Cancel (esc)", "Esc")]
  if (name === "Agents" && b.editor) return b.editor_clear_confirm ? [char("Clear", "y"), char("Keep text", "n")] : [button("Done (esc)", "Esc"), button("Clear (ctrl+x)", { Char: "x" }, 2)]
  if (b.tool_picker) return [char("Toggle", " "), enter("Done"), back]
  if (b.model_picker) return [enter("Choose"), back]
  if (["Agents", "Mcp"].includes(name)) return mode === "create" || mode === "edit"
    ? [enter("Edit / toggle"), { label: "Save (s)", action: "Save", primary: true }, back]
    : [enter("Edit"), char("Delete", "d"), back]
  if (name === "Extensions") return mode === "detail" ? [enter("Open screen"), char("Global activation", "g"), char("On demand", "o"), char("Uninstall", "u"), back] : [enter("Details"), back]
  if (name === "ExtStore") return mode === "install_confirm" ? [ { ...char("Install", "y"), disabled: !!b.installing }, back ] : mode === "detail" ? [char("Install", "i"), back] : [enter("Details"), char("Refresh", "r"), back]
  if (name === "ExtScreen") return [{ ...enter("Select"), disabled: !!b.waiting }, back]
  if (name === "Security") {
    const pane = String(b.pane || (b.health_view ? "deps" : "tools"))
    const paneJump = (label: string, id: string, primary = false): ActionDescriptor => ({
      label,
      primary,
      action: { Replace: { field: "pane", value: id } },
    })
    const tabs = [
      paneJump("Dashboard", "tools", pane === "tools"),
      paneJump("Images", "images", pane === "images"),
      paneJump("Deps", "deps", pane === "deps"),
      paneJump("Refused", "refused", pane === "refused"),
    ]
    if (pane === "images") {
      return [
        ...tabs,
        enter("Activate"),
        char("Pull", "p"),
        char("Remove", "x"),
        char("Refresh list", "r"),
        back,
      ]
    }
    if (pane === "deps") {
      return [...tabs, char("Install", "i"), char("Restart", "r"), back]
    }
    if (pane === "refused") {
      return [...tabs, char("Refresh", "u"), char("Restart", "r"), back]
    }
    return [
      ...tabs,
      enter("Toggle"),
      char("Runtime", "c"),
      char("Refresh", "u"),
      char("Restart", "r"),
      back,
    ]
  }
  if (name === "Remote") {
    const sub = String(b.sub)
    if (sub.endsWith(":edit")) return [enter("Edit"), { label: "Save (s)", action: "Save", primary: true }, back]
    if (sub.startsWith("new:") || sub.startsWith("resume:")) return [enter(sub.startsWith("resume:") ? "Open" : "Connect"), back]
    return [enter("Connect"), char("Edit", "e"), char("Delete", "d"), char("Import", "i"), back]
  }
  if (name === "Notifications") return b.confirm_clear ? [char("Clear permanently", "y"), back] : [button("Scope (tab)", "Tab"), button("Severity (ctrl+f)", { Char: "f" }, 2), enter("Details / read"), button("Read all (ctrl+r)", { Char: "r" }, 2), button("Clear (ctrl+x)", { Char: "x" }, 2), back]
  if (name === "Usage") return [button("Scope (tab)", "Tab"), char("Today", "1"), char("Week", "2"), char("Year", "3"), char("Metric", "m"), back]
  if (name === "SessionHub") return b.armed ? [enter("Confirm"), back] : [button("Switch (tab)", "Tab"), enter("Open"), button("Remove (ctrl+x)", { Char: "x" }, 2), back]
  if (name === "Help") return [enter("Run command"), back]
  if (name === "Effort") return [enter("Set effort"), back]
  if (name === "Attachments" && b.editor) return [button("Done (esc)", "Esc")]
  return [enter("Continue"), back]
}

/** Resolve pointer targets by stable identity at the snapshot boundary. */
export function identifiedSelection(name: string, body: Record<string, unknown>, selection: { group: string; index: number; id?: string | null }) {
  // Contracts are generated from Rust; legacy/missing additive fields remain optional at this boundary.
  const b = body as Record<string, unknown> & Partial<import("./data.ts").SettingsSnapshot & import("./data.ts").AgentsSnapshot & import("./data.ts").McpSnapshot & import("./data.ts").ExtensionsSnapshot & import("./data.ts").ExtStoreSnapshot & import("./data.ts").ExtScreenSnapshot & import("./data.ts").RemoteSnapshot>
  const i = selection.index
  let id: string | undefined
  let group = selection.group
  if (name === "Settings" && group === "active") {
    if (b.page === "providers") id = b.providers?.[i]?.uuid
    else if (b.page === "oauth" && b.oauth_flow?.kind === "idle") id = b.oauth_drafts?.[i]?.uuid
    else if (b.page === "models" && i >= 5) id = b.models?.filter((m: { session_only: boolean }) => b.model_filter === "all" || !b.model_filter || (b.model_filter === "local" ? m.session_only : !m.session_only))[i - 5]?.uuid
  } else if (name === "Agents" && (!b.mode || b.mode === "browse")) id = b.agents?.[i]?.name
  else if (name === "Mcp" && (!b.mode || b.mode === "browse")) id = b.servers?.[i]?.uuid
  else if (name === "Extensions") { if (b.mode === "detail") group = "screens"; else id = b.rows?.[i]?.id }
  else if (name === "ExtStore") id = b.rows?.[i]?.id
  else if (name === "ExtScreen") id = (b.screen as { body?: { t: string; items?: { id?: string }[] }[] } | null)?.body?.filter((n) => n.t === "menu").flatMap((n) => n.items?.filter((item) => item.id) ?? [])?.[i]?.id
  else if (name === "Remote" && String(b.sub).endsWith(":browse")) id = b.hosts?.[b.filtered?.[i] ?? i]?.id
  return { ...selection, group, id: selection.id ?? id }
}

function oauthControls(kind: string): ActionDescriptor[] {
  if (["codex_wait","kilo_wait"].includes(kind)) return [char("Open browser","o"),char("Copy URL","c"),back]
  if (kind === "starting") return [back]
  if (kind === "failed") return [enter("Try again"),back]
  return [enter("Connect"),back]
}
