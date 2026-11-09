import type { MenuData } from "./data.ts"
import { identifiedSelection, menuControls, replaceField, select } from "./actions.ts"
import { useRef, type ReactNode } from "react"
import { usePaste } from "@opentui/react"
import type { Theme } from "../theme.ts"
import { Controls, DraftContext, NativeDraftContext, InputSuspendedContext, Fullscreen, Overlay, nativeMenuInput } from "./bits.tsx"
import { agentFields, mcpFields, agentsView, extScreenView, extensionsView, mcpView, remoteView, storeView } from "./manage.tsx"
import { securityView } from "./security.tsx"
import {
  attachmentsView,
  bashView,
  effortView,
  helpView,
  modelView,
  notificationsView,
  skillView,
  todoView,
  usageView,
} from "./panels.tsx"
import { placeOf, wideEnough } from "./place.ts"
import { modeParts, type Rec } from "./read.ts"
import { settingsView } from "./settings.tsx"
import { keyInputView, onboardProviderView, onboardView, sessionHubView, sessionPickerView } from "./setup.tsx"

type Built = { title: ReactNode; hint: ReactNode; children: ReactNode; corner?: ReactNode }

export function ModeSurface({
  mode,
  disabled = false,
  theme,
  columns,
  onMove,
  send,
}: {
  mode: unknown
  disabled?: boolean
  theme: Theme
  columns: number
  onMove: (index: number) => void
  send: (request: unknown) => void
}) {
  usePaste((event) => {
    if (event.defaultPrevented || nativeMenuInput.current) return
    event.preventDefault()
    send({ Paste: { text: new TextDecoder().decode(event.bytes) } })
  })
  const parts = modeParts(mode)
  const draftKey = `${parts?.name}:${parts?.body.page ?? ""}:${parts?.body.mode ?? ""}:${parts?.body.sub ?? ""}:${parts?.body.step ?? ""}:${parts?.body.page === "quick" && !parts?.body.search_editor ? parts?.body.editing : ""}:${parts?.body.prov_replace ?? ""}:${(parts?.body.model_modal as {uuid?:string} | undefined)?.uuid ?? ""}`
  const store = useRef({key: draftKey, drafts: new Map()})
  if (store.current.key !== draftKey) store.current = {key: draftKey, drafts: new Map()}
  const search = parts?.body.search_editor as {provider?: string} | undefined
  const searchProvider = useRef(search?.provider)
  if (searchProvider.current !== search?.provider) { store.current = {key: draftKey, drafts: new Map()}; searchProvider.current = search?.provider }
  const modelProvider = (parts?.body.model_modal as {provider_idx?: number} | undefined)?.provider_idx
  const previousModelProvider = useRef(modelProvider)
  if (previousModelProvider.current !== modelProvider) {
    store.current.drafts.delete("Model")
    previousModelProvider.current = modelProvider
  }
  const originalSend = send
  send = (request) => {
    if (request && typeof request === "object" && !("MenuReplace" in request)) nativeMenuInput.flush?.()
    if (parts && request && typeof request === "object" && "MenuSelect" in request) {
      const selection = (request as { MenuSelect: { group: string; index: number; id?: string } }).MenuSelect
      originalSend({ MenuSelect: identifiedSelection(parts.name, parts.body, selection) })
    } else originalSend(request)
  }
  const move = placeOf(mode) === "full" ? (index: number) => select(send, index) : onMove
  const built = build(mode, theme, wideEnough(columns), move, send)
  if (!built) return null
  const place = placeOf(mode)
  if (place === "overlay") {
    return (
      <Overlay title={built.title} hint={built.hint} corner={built.corner} theme={theme}>
        {built.children}
      </Overlay>
    )
  }
  return (
    <Fullscreen status={menuStatus(parts?.name, parts?.body)} contentHeight={compactFormHeight(parts?.name, parts?.body)} title={built.title} hint={typeof built.hint === "string" ? <Controls actions={menuControls(parts!.name, parts!.body)} theme={theme} send={send} /> : built.hint} theme={theme}>
      <NativeDraftContext.Provider value={store.current.drafts}><InputSuspendedContext.Provider value={disabled}><DraftContext.Provider value={(field, value) => replaceField(send, field, value)}>{built.children}</DraftContext.Provider></InputSuspendedContext.Provider></NativeDraftContext.Provider>
    </Fullscreen>
  )
}

function build(mode: unknown, theme: Theme, wide: boolean, onMove: (index: number) => void, send: (request: unknown) => void): Built | null {
  const parts = modeParts(mode)
  if (!parts || !placeOf(mode)) return null
  const { name, body } = parts
  const view = viewOf(name, body, theme, wide, onMove, send)
  if (view) return view
  return { title: name, hint: "esc back", children: <text fg={theme.dim}> nothing to show</text> }
}

function viewOf(name: string, body: Rec, theme: Theme, wide: boolean, onMove: (index: number) => void, send: (request: unknown) => void): Built | null {
  switch (name) {
    case "Settings":
      return settingsView(body as MenuData["Settings"] & Rec, theme, wide, onMove, send)
    case "Help":
      return helpView(body as MenuData["Help"] & Rec, theme, onMove)
    case "Effort":
      return effortView(body as MenuData["Effort"] & Rec, theme, onMove)
    case "Model":
      return modelView(body, theme, onMove, send)
    case "Todo":
      return todoView(body, theme, wide, onMove)
    case "Bash":
      return bashView(body, theme, wide, onMove)
    case "Skill":
      return skillView(body, theme, onMove)
    case "Attachments":
      return attachmentsView(body, theme, onMove)
    case "Notifications":
      return notificationsView(body as MenuData["Notifications"] & Rec, theme, onMove, send)
    case "Usage":
      return usageView(body as MenuData["Usage"] & Rec, theme, wide, send)
    case "Agents":
      return agentsView(body as MenuData["Agents"] & Rec, theme, wide, onMove, send)
    case "Mcp":
      return mcpView(body as MenuData["Mcp"] & Rec, theme, wide, onMove, send)
    case "Extensions":
      return extensionsView(body as MenuData["Extensions"] & Rec, theme, wide, onMove, send)
    case "ExtStore":
      return storeView(body as MenuData["ExtStore"] & Rec, theme, wide, onMove)
    case "ExtScreen":
      return extScreenView(body as MenuData["ExtScreen"] & Rec, theme, onMove)
    case "Security":
      return securityView(body as MenuData["Security"] & Rec, theme, onMove, send)
    case "Remote":
      return remoteView(body as MenuData["Remote"] & Rec, theme, wide, onMove, send)
    case "Onboard":
      return onboardView(body as MenuData["Onboard"] & Rec, theme, onMove)
    case "OnboardProvider":
      return onboardProviderView(body as MenuData["OnboardProvider"] & Rec, theme, onMove, send)
    case "KeyInput":
      return keyInputView(body as MenuData["KeyInput"] & Rec, theme, onMove, send)
    case "SessionHub":
      return sessionHubView(body as MenuData["SessionHub"] & Rec, theme, wide, send)
    case "SessionPicker":
      return sessionPickerView(body as MenuData["Picker"] & Rec, theme, onMove)
    default:
      return null
  }
}

/** Keep short form actions adjacent; long forms retain a bounded browsing area. */
function compactFormHeight(name: string | undefined, b: Rec | undefined): number | undefined {
  if (!b) return
  if (name === "Agents" && ["create", "edit"].includes(String(b.mode)) && !b.editor && !b.tool_picker && !b.model_picker) return agentFields(String(b.mode)).length * 3 + (b.mode === "create" ? 1 : 0)
  if (name === "Mcp" && ["create", "edit"].includes(String(b.mode))) return mcpFields(String(b.draft_transport) || "stdio").length * 3
  if (name === "Remote" && String(b.sub).endsWith(":edit")) return 15 + ((b.editor as {error?: string})?.error ? 1 : 0)
  if (name === "KeyInput" && b.step === 0) return 6
}

/** A reserved footer row prevents transient work notices from moving content. */
function menuStatus(name: string | undefined, b: Rec | undefined): string | undefined {
  if (!b) return
  if (name === "Settings" && b.page === "model_form") return (b.model_modal as {endpoints_loading?:boolean})?.endpoints_loading ? "Loading routes…" : b.model_loading ? "Loading models…" : ""
  if (name === "ExtStore") return b.installing ? "Installing…" : b.loading || b.detail_loading ? "Loading catalogue…" : ""
  if (name === "ExtScreen") return b.waiting ? "Loading…" : ""
  if (name === "Security") return b.images_busy ? "Working on image…" : ""
}
