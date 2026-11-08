import { agentFields, mcpFields } from "./manage.tsx"
import { modeParts, num, rec, str, type Rec } from "./read.ts"

/** Where a daemon mode is drawn. Chat, quit, rewind, and loading are handled elsewhere. */
export function placeOf(mode: unknown): "overlay" | "full" | null {
  const parts = modeParts(mode)
  if (!parts) return null
  const { name, body } = parts
  if (name === "Chat" || name === "QuitConfirm" || name === "MessageRewind" || name === "Loading") return null
  if (name === "Settings") {
    const page = strPage(body.page)
    return page === "" || page === "menu" ? "overlay" : "full"
  }
  if (name === "Extensions") return body.mode === "use" ? "overlay" : "full"
  if (name === "Attachments") return body.editor ? "full" : "overlay"
  if (name === "Effort" || name === "Model" || name === "Bash" || name === "Todo" || name === "Skill") return "overlay"
  return "full"
}

function strPage(value: unknown): string {
  return typeof value === "string" ? value : ""
}

export function wideEnough(columns: number): boolean {
  return columns >= 80
}

/** The row a click should measure its arrow steps from. */
export function selectedOf(mode: unknown): number {
  const parts = modeParts(mode)
  if (!parts) return 0
  const body = parts.body
  switch (parts.name) {
    case "Settings":
      return settingsSelected(body)
    case "Help":
    case "Skill":
    case "Effort":
    case "SessionPicker":
    case "Bash":
    case "Todo":
    case "Attachments":
      return num(body.selected)
    case "Model":
      return num(body.cursor)
    case "Notifications":
      return num(body.cursor)
    case "Agents":
      if (body.tool_picker) return num(rec(body.tool_picker).cursor)
      if (body.model_picker) return num(rec(body.model_picker).cursor)
      if (body.mode === "edit" || body.mode === "create") {
        return Math.max(0, agentFields(str(body.mode)).indexOf(str(body.field)))
      }
      return num(body.list_sel)
    case "Mcp":
      if (body.mode === "edit" || body.mode === "create") {
        return Math.max(0, mcpFields(str(body.draft_transport) || "stdio").indexOf(str(body.field)))
      }
      return num(body.list_sel)
    case "Extensions":
    case "ExtStore":
      return num(body.list_sel)
    case "ExtScreen":
      return num(body.menu_cursor)
    case "Security": {
      const pane = str(body.pane) || "tools"
      if (pane === "refused") return num(body.refused_selected)
      if (pane === "images") return num(body.images_selected)
      return num(body.selected)
    }
    case "Remote": {
      const view = str(body.sub).split(":")[1] ?? ""
      if (view === "session_hub") return num(body.session_selected)
      if (view === "edit") {
        const fields = ["name", "user", "host", "port", "key_path"]
        return Math.max(0, fields.indexOf(str(rec(body.editor).focused)))
      }
      return num(body.selected)
    }
    case "Onboard":
      return num(body.cursor)
    case "KeyInput":
      return num(body.step) === 1 ? num(body.result_sel) : num(body.field)
    case "OnboardProvider":
      return body.step === "model_select" ? num(body.result_sel) : num(rec(body.oauth_flow).cursor)
    case "SessionHub":
      return num(body.focus_cooking ? body.cooking_selected : body.history_selected)
    default:
      return num(body.selected || body.list_sel || body.cursor)
  }
}

function settingsSelected(body: Rec): number {
  const page = str(body.page) || "quick"
  if (body.search_editor) {
    return Math.max(0, ["built_in", "firecrawl", "tavily", "exa"].indexOf(str(rec(body.search_editor).provider)))
  }
  if (body.picker) return num(rec(body.picker).sel)
  if (body.list_editing) return num(body.list_sel)
  if (page === "quick") return str(body.quick_focus) === "general" ? num(body.field) : num(body.palette_sel)
  if (page === "menu") return num(body.menu_sel)
  if (page === "appearance") return num(body.palette_sel)
  if (page === "providers") return num(body.prov_sel)
  if (page === "provider_form") return num(rec(body.prov_modal).field)
  if (page === "oauth") {
    const flow = rec(body.oauth_flow)
    return str(flow.kind) === "pick" ? num(flow.cursor) : num(body.oauth_sel)
  }
  if (page === "models") return num(body.model_sel)
  if (page === "model_form") {
    const modal = rec(body.model_modal)
    if (modal.role_picker) return num(rec(modal.role_picker).cursor)
    return num(modal.field)
  }
  return num(body.field)
}
