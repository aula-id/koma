import type { MenuData } from "./data.ts"
import type { ReactNode } from "react"
import type { Theme } from "../theme.ts"
import { Action, Document, FieldList, Lines, Note, OVERLAY_BODY, Panel, Paragraphs, Split, sendKey, type Line } from "./bits.tsx"

import { editorFrom, TextEditor } from "./editor.tsx"
import { clip, flag, num, rec, rows, str, strings, type Rec } from "./read.ts"

export function agentsView(body: MenuData["Agents"] & Rec, theme: Theme, wide: boolean, onMove: (index: number) => void, send: (request: unknown) => void) {
  const editor = body.editor ? editorFrom(body.editor) : null
  if (editor) {
    return {
      title: `agent · ${str(Array.isArray(body.editor) ? body.editor[0] : "edit")}`,
      hint: body.editor_clear_confirm ? "clear this text?  y / n" : "arrows · enter newline · esc close",
      children: body.editor_clear_confirm ? <Note text="Clear the entire field? Choose Clear or Keep text." color={theme.warn} theme={theme} /> : <TextEditor body={editor} theme={theme} />,
    }
  }
  if (body.tool_picker) return pickerView("tools", rec(body.tool_picker), theme, onMove, true)
  if (body.model_picker) return pickerView("model", rec(body.model_picker), theme, onMove, false)
  const mode = str(body.mode) || "browse"
  const agents = rows(body.agents)
  const selected = num(body.list_sel)
  const lines: Line[] = agents.map((row) => ({ label: str(row.name), detail: clip(str(row.description), 64), badge: { text: str(row.source), role: "info" } }))
  if (mode === "edit" || mode === "create") {
    const fields = agentFields(mode)
    const form: Line[] = fields.map((field) => ({
      label: field,
      detail: agentDraft(body, field),
    }))
    return {
      title: `agents · ${mode}`,
      hint: body.editing ? "enter done editing · esc finish editing" : "↑↓ field · enter edit · s save · esc back",
      children: <box flexDirection="column" flexGrow={1} minHeight={0}>{mode === "create" ? <box flexDirection="row" flexShrink={0}><Action label="Session scope" primary={body.create_scope === "session"} theme={theme} onPress={() => send({ MenuSelect: { group: "scope", index: 0 } })} /><Action label="Global scope" primary={body.create_scope === "global"} theme={theme} onPress={() => send({ MenuSelect: { group: "scope", index: 1 } })} /></box> : null}<FieldList editing={flag(body.editing) || flag(body.editing_field)} rows={form} selected={Math.max(0, fields.indexOf(str(body.field)))} theme={theme} onPress={onMove} /></box>,
    }
  }
  if (mode === "delete_confirm") {
    return {
      title: "agents",
      hint: "y delete · n cancel",
      children: <Note text={`delete ${str(agents[selected]?.name)}?`} theme={theme} color={theme.warn} />,
    }
  }
  const detail = draftLines(agents[selected])
  return {
    title: "agents",
    hint: "↑↓ · enter edit · n new · d delete · esc close",
    children: managementList("Agents", theme, lines, selected, onMove, <Action label="+ New (n)" theme={theme} onPress={() => sendKey(send, { Char: "n" })} />),
  }
}

export function mcpView(body: MenuData["Mcp"] & Rec, theme: Theme, wide: boolean, onMove: (index: number) => void, send: (request: unknown) => void) {
  const mode = str(body.mode) || "browse"
  const servers = rows(body.servers)
  const selected = num(body.list_sel)
  const status = rec(body.status)
  const errors = rec(body.errors)
  const lines: Line[] = servers.map((row) => ({
    label: str(row.name),
    detail: str(row.transport),
    toggle: flag(row.enabled),
    badge: { text: errors[str(row.uuid)] ? str(errors[str(row.uuid)]) : Object.hasOwn(status, str(row.uuid)) ? `connected · ${num(status[str(row.uuid)])} tools` : flag(row.enabled) ? body.available === false ? "manager unavailable" : "awaiting status" : "disabled", role: errors[str(row.uuid)] ? "error" : Object.hasOwn(status, str(row.uuid)) ? "success" : "dim" },
  }))
  if (mode === "delete_confirm") {
    return {
      title: "mcp",
      hint: "y delete · n cancel",
      children: <Note text={`delete ${str(servers[selected]?.name)}?`} theme={theme} color={theme.warn} />,
    }
  }
  if (mode === "edit" || mode === "create") {
    const transport = str(body.draft_transport) || "stdio"
    const fields = mcpFields(transport)
    const form: Line[] = fields.map((field) => ({
      label: field,
      detail: field === "enabled" ? undefined : mcpDraft(body, field),
      toggle: field === "enabled" ? flag(body.draft_enabled) : undefined,
    }))
    return {
      title: `mcp · ${mode}`,
      hint: body.editing ? "enter done editing · esc finish editing" : "↑↓ field · enter edit · space toggle · s save · esc back",
      children: <FieldList editing={flag(body.editing) || flag(body.editing_field)} rows={form} selected={Math.max(0, fields.indexOf(str(body.field)))} theme={theme} onPress={onMove} />,
    }
  }
  const current = servers[selected]
  const detail = [str(current?.name) || "no servers", str(current?.command || current?.url), str(current?.transport)]
  return {
    title: "mcp",
    hint: "↑↓ · enter edit · n new · d delete · esc close",
    children: managementList("MCP servers", theme, lines, selected, onMove, <Action label="+ New (n)" theme={theme} onPress={() => sendKey(send, { Char: "n" })} />),
  }
}

export function extensionsView(body: MenuData["Extensions"] & Rec, theme: Theme, wide: boolean, onMove: (index: number) => void, send: (request: unknown) => void) {
  const mode = str(body.mode) || "browse"
  const list = rows(body.rows)
  const selected = num(body.list_sel)
  const lines: Line[] = list.map((row) => ({
    label: str(row.name),
    detail: str(row.version),
    badge: { text: flag(row.running) ? "running" : flag(row.active) ? "active" : "not loaded", role: flag(row.running) || flag(row.active) ? "success" : "dim" },
  }))
  if (mode === "use") {
    return {
      title: "use extension",
      hint: "↑↓ · enter load · x unload · esc close",
      children: <Lines rows={lines.map((line, index) => ({ ...line, toggle: flag(list[index]?.active), toggleLayout: "before" as const }))} selected={selected} theme={theme} visible={OVERLAY_BODY} onPress={onMove} />,
    }
  }
  if (mode === "uninstall_confirm") {
    return {
      title: "extensions",
      hint: "y uninstall · n cancel",
      children: <Note text={`uninstall ${str(list[selected]?.name)}?`} theme={theme} color={theme.warn} />,
    }
  }
  const current = list[selected]
  const screens = rows(current?.tui_screens)
  const detail = current
    ? [
        `${str(current.name)} · ${str(current.version)}`,
        str(current.description),
        `${str(current.kind)} · ${str(current.tier)}`,
        flag(current.active) ? "active this session" : "not loaded",
        `tools ${str(current.tools)}  panels ${str(current.panels)}  agents ${str(current.sub_agents)}  models ${str(current.models)}`,
        `Activation  ${str(current.activation) || "manual"} · ${flag(current.enabled) ? "enabled" : "disabled"}`,
        `Workspace  ${str(current.workspace_dir) || "none"}`,
        `Grants  ${strings(current.granted).join(", ") || "none"}`,
        ...screens.map((row, index) => `${index === num(body.screen_sel) && mode === "detail" ? "› " : ""}${str(row.title)}`),
      ]
    : ["no extensions"]
  return {
    title: mode === "detail" ? "extension" : "extensions",
    hint: mode === "detail" ? "↑↓ screens · enter open · u uninstall · esc back" : "↑↓ · enter detail · esc close",
    children: (
      <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0}>
        {str(body.error) ? <Note text={str(body.error)} theme={theme} color={theme.error} /> : null}
        {mode === "detail" ? <Document theme={theme} target={`extension-screen-${num(body.screen_sel)}`}><text fg={theme.accent}>{str(current?.name)}</text>{detail.slice(0, detail.length - screens.length).map((line, index) => <text key={index} fg={theme.fg}>{line}</text>)}<text fg={theme.info}>Extension screens</text>{screens.length ? screens.map((screen, index) => <box id={`extension-screen-${index}`} key={str(screen.id)} height={1} onMouseDown={() => { onMove(index); sendKey(send, "Enter") }}><text fg={index === num(body.screen_sel) ? theme.accent : theme.fg}>{`${index === num(body.screen_sel) ? "›" : " "} ${str(screen.title)}`}</text></box>) : <text fg={theme.dim}>No terminal screens contributed.</text>}</Document> : <Lines rows={lines} selected={selected} theme={theme} onPress={onMove} />}
      </box>
    ),
  }
}

export function storeView(body: MenuData["ExtStore"] & Rec, theme: Theme, wide: boolean, onMove: (index: number) => void) {
  const mode = str(body.mode) || "browse"
  const list = rows(body.rows)
  const selected = num(body.list_sel)
  const lines: Line[] = list.map((row) => ({
    label: str(row.name),
    detail: str(row.latest_version),
    badge: { text: flag(row.installed) ? "installed" : str(row.tier), role: flag(row.installed) ? "success" : "info" },
  }))
  if (mode === "install_confirm") {
    return {
      title: "store",
      hint: "y install · n cancel",
      children: (
        <box flexDirection="column">
          <Note text={`install ${str(list[selected]?.name)}?`} theme={theme} />
          {str(body.install_error) ? <Note text={str(body.install_error)} theme={theme} color={theme.error} /> : null}
          <Note text={flag(body.komarun_connected) ? "signed in" : "sign in on koma.run first"} theme={theme} />
        </box>
      ),
    }
  }
  const detailBody = rec(body.detail)
  const detail = mode === "detail"
    ? [
        str(detailBody.description),
        `tools ${str(detailBody.contributes_tools)}  panels ${str(detailBody.contributes_panels)}`,
        `Requirements  ${strings(detailBody.requires).join(", ") || "none"}`,
        `Versions  ${strings(detailBody.versions).join("  ") || "not available"}`,
        `Models ${num(detailBody.contributes_models)}  Agents ${num(detailBody.contributes_sub_agents)}`,
        flag(body.komarun_connected) ? "Signed in to koma.run" : "Sign in to koma.run to install authenticated products",
        str(body.detail_error),
      ].filter(Boolean)
    : [body.loading ? "loading the catalogue…" : "", str(body.error)].filter(Boolean)
  return {
    title: mode === "detail" ? "store · detail" : "store",
    hint: mode === "detail" ? "i install · esc back" : "↑↓ · enter detail · r refresh · esc close",
    children: mode === "detail" ? <Document theme={theme}><text fg={theme.accent}>{str(list[selected]?.name)}</text>{detail.map((line, index) => <text key={index} fg={index === detail.length - 1 && body.detail_error ? theme.error : theme.fg}>{line}</text>)}</Document> : <box flexDirection="column" flexGrow={1} minHeight={0}>{body.error ? <Note text={str(body.error)} color={theme.error} theme={theme} /> : null}<Lines rows={lines} selected={selected} theme={theme} onPress={onMove} /></box>,
  }
}

function progressBar(done: number, total: number, width = 18): string {
  const t = Math.max(0, total)
  const d = Math.max(0, Math.min(done, t || done))
  const pct = t > 0 ? d / t : done > 0 ? 1 : 0
  const fill = Math.round(pct * width)
  return `${"█".repeat(fill)}${"░".repeat(Math.max(0, width - fill))} ${t ? `${d}/${t}` : `${d}`}`
}

function securityMeter({ label, done, total, theme, color }: { label: string; done: number; total: number; theme: Theme; color?: string }) {
  const bar = progressBar(done, total, 16)
  const fg = color ?? theme.accent
  return (
    <box height={1} flexShrink={0} flexDirection="row" paddingLeft={1} paddingRight={1}>
      <text fg={theme.dim}>{label.padEnd(10)}</text>
      <text fg={fg}>{bar}</text>
    </box>
  )
}

export function securityView(body: MenuData["Security"] & Rec, theme: Theme, onMove: (index: number) => void, send: (request: unknown) => void) {
  const status = rec(body.status)
  const tools = rows(status.tools)
  const inactive = new Set(strings(body.inactive))
  const health = rows(body.install_health)
  const refused = rows(status.refused)
  const images = rows(body.images)
  const pane = str(body.pane) || (flag(body.health_view) ? "deps" : "tools")
  const setPane = (id: string) => send({ MenuReplace: { field: "pane", value: id } })
  const healthSorted = [...health].sort((a, b) => num(a.tier) - num(b.tier))
  const lines: Line[] =
    pane === "deps"
      ? healthSorted.map((row) => ({
        label: str(row.name) || str(row.key),
        detail: `tier ${num(row.tier)} · ${str(row.method)}`,
        badge: { text: flag(row.present) ? "present" : "missing", role: flag(row.present) ? "success" : "warn" },
      }))
      : pane === "refused"
        ? refused.map((row) => ({ label: str(row.name), detail: str(row.reason), badge: { text: "refused", role: "warn" } }))
        : pane === "images"
          ? images.map((row) => ({
            label: `${flag(row.active) ? "* " : "  "}${str(row.id)}`,
            detail: `${str(row.size_label) || "—"} · ${str(row.blurb)}`,
            badge: {
              text: flag(row.active) ? "active" : flag(row.local) ? "local" : "remote",
              role: flag(row.active) ? "success" : flag(row.local) ? "info" : "dim",
            },
          }))
          : [
            { label: "Security daemon", toggle: flag(status.running), badge: { text: flag(status.running) ? "running" : "stopped", role: flag(status.running) ? "success" : "dim" } },
            { label: "YOLO mode", toggle: flag(body.yolo_armed), badge: { text: flag(body.yolo_armed) ? "armed" : "locked", role: flag(body.yolo_armed) ? "warn" : "dim" } },
          ]
  const selected =
    pane === "deps"
      ? num(body.health_selected)
      : pane === "refused"
        ? num(body.refused_selected)
        : pane === "images"
          ? num(body.images_selected)
          : num(body.selected)
  const runtime = str(status.runtime) || (flag(status.runtime_ok) ? "docker ready" : "runtime unknown")
  const lane = str(body.lane_label) || "none"
  const refusedN = refused.length
  const catalogReady = num(status.catalog_ready)
  const catalogTotal = num(status.catalog_total) || tools.length
  const depsPresent = health.filter((row) => flag(row.present)).length
  const depsTotal = health.length
  const imagesLocal = images.filter((row) => flag(row.local)).length
  const imagesActive = str(images.find((row) => flag(row.active))?.id) || "—"
  const toolsOn = tools.filter((row) => !inactive.has(str(row.name))).length
  const paneTabs: { id: string; label: string }[] = [
    { id: "tools", label: "Dashboard" },
    { id: "images", label: "Images" },
    { id: "deps", label: "Deps" },
    { id: "refused", label: "Refused" },
  ]
  const paneTitle =
    pane === "deps" ? "Dependencies" : pane === "refused" ? "Refused" : pane === "images" ? "Guest images" : "Controls"
  const hint =
    pane === "images"
      ? "click panes · scroll list · activate / pull / remove in footer"
      : pane === "deps"
        ? "click panes · scroll · install in footer"
        : "click panes · click rows · footer actions · esc back"
  return {
    title: "security",
    hint,
    children: (
      <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0}>
        {/* clickable pane tabs */}
        <box flexDirection="row" flexShrink={0} flexWrap="wrap" columnGap={1} paddingLeft={1} paddingBottom={1}>
          {paneTabs.map((tab) => {
            const on = pane === tab.id || (pane === "tools" && tab.id === "tools")
            return (
              <box
                key={tab.id}
                height={1}
                flexShrink={0}
                paddingLeft={1}
                paddingRight={1}
                backgroundColor={on ? theme.selBg : theme.panel}
                onMouseDown={() => setPane(tab.id)}
              >
                <text fg={on ? theme.selFg : theme.fg}>{on ? `▸ ${tab.label}` : `  ${tab.label}`}</text>
              </box>
            )
          })}
        </box>

        {/* stats dashboard */}
        <box flexShrink={0} flexDirection="column" border borderStyle="rounded" borderColor={theme.dim} title=" status " titleColor={theme.dim} marginBottom={1}>
          <box height={1} paddingLeft={1} flexDirection="row">
            <text fg={flag(status.running) ? theme.success : theme.dim}>{flag(status.running) ? "● running" : "○ stopped"}</text>
            <text fg={theme.dim}>{` · ${flag(status.installed) ? "installed" : "not installed"} · ${runtime} · lane ${lane}`}</text>
          </box>
          <box height={1} paddingLeft={1} flexDirection="row">
            <text fg={theme.dim}>{`yolo ${flag(body.yolo_armed) ? "armed" : "locked"} · boundary ${flag(status.boundary_up) ? "up" : "down"} · active image ${imagesActive || "—"}`}</text>
          </box>
          {securityMeter({ label: "catalog", done: catalogReady, total: catalogTotal, theme, color: theme.success })}
          {securityMeter({ label: "tools on", done: toolsOn, total: tools.length || toolsOn, theme, color: theme.accent })}
          {depsTotal > 0 ? securityMeter({ label: "deps ok", done: depsPresent, total: depsTotal, theme, color: depsPresent === depsTotal ? theme.success : theme.warn }) : null}
          {images.length > 0 ? securityMeter({ label: "images", done: imagesLocal, total: images.length, theme, color: theme.info }) : null}
          {refusedN > 0 ? securityMeter({ label: "refused", done: refusedN, total: refusedN, theme, color: theme.error }) : null}
          {!flag(status.installed) ? <Note text="run: koma --security-install  (minimal guest only)" theme={theme} color={theme.warn} /> : null}
        </box>

        {pane === "deps" && healthSorted[selected]?.hint ? (
          <Note text={`${str(healthSorted[selected]?.method)} · ${str(healthSorted[selected]?.hint)}`} theme={theme} color={theme.info} />
        ) : null}
        {pane === "refused" && !refused.length ? <Note text={flag(status.running) ? "none — every descriptor passed" : "no data (daemon stopped)"} theme={theme} /> : null}
        {pane === "images" && flag(body.images_busy) ? <Note text="pulling image…" theme={theme} color={theme.info} /> : null}
        {pane === "images" && images[selected] ? (
          <box flexDirection="row" flexShrink={0} columnGap={1} paddingLeft={1} paddingBottom={1}>
            <Action label="Activate" primary theme={theme} onPress={() => sendKey(send, "Enter")} />
            <Action label="Pull" theme={theme} onPress={() => sendKey(send, { Char: "p" })} />
            <Action label="Remove" theme={theme} onPress={() => sendKey(send, { Char: "x" })} />
            <Action label="Refresh" theme={theme} onPress={() => sendKey(send, { Char: "r" })} />
          </box>
        ) : null}
        {pane === "tools" ? (
          <box flexDirection="row" flexShrink={0} columnGap={1} paddingLeft={1} paddingBottom={1}>
            <Action label="Start/Stop" primary theme={theme} onPress={() => sendKey(send, "Enter")} />
            <Action label="Runtime" theme={theme} onPress={() => sendKey(send, { Char: "c" })} />
            <Action label="Refresh" theme={theme} onPress={() => sendKey(send, { Char: "u" })} />
            <Action label="Restart" theme={theme} onPress={() => sendKey(send, { Char: "r" })} />
          </box>
        ) : null}
        {pane === "deps" ? (
          <box flexDirection="row" flexShrink={0} columnGap={1} paddingLeft={1} paddingBottom={1}>
            <Action label="Install dep" primary theme={theme} onPress={() => sendKey(send, { Char: "i" })} />
            <Action label="Restart" theme={theme} onPress={() => sendKey(send, { Char: "r" })} />
          </box>
        ) : null}
        {body.health_fetching ? <Note text="checking dependencies…" theme={theme} /> : null}

        <Panel title={paneTitle} hot theme={theme}>
          <Lines rows={lines} selected={selected} theme={theme} onPress={onMove} rail />
        </Panel>
      </box>
    ),
  }
}

export function remoteView(body: MenuData["Remote"] & Rec, theme: Theme, wide: boolean, onMove: (index: number) => void, send: (request: unknown) => void) {
  const sub = str(body.sub)
  const view = sub.split(":")[1] || "browse"
  const hosts = rows(body.hosts)
  const filtered = Array.isArray(body.filtered) ? body.filtered.map(num) : hosts.map((_, index) => index)
  const selected = num(body.selected)
  const lines: Line[] = filtered.map((index) => {
    const row = hosts[index] ?? {}
    return { label: str(row.name) || str(row.host), detail: `${str(row.user)}@${str(row.host)}:${str(row.port)}`, badge: { text: flag(row.connected) ? "previously connected" : "saved", role: "info" } }
  })
  if (view === "delete_confirm") {
    return {
      title: "remote",
      hint: "y delete · n cancel",
      children: <Note text={`Delete ${str(hosts[filtered[selected] ?? selected]?.name) || str(hosts[filtered[selected] ?? selected]?.host)}?`} theme={theme} color={theme.warn} />,
    }
  }
  if (view === "edit") {
    const editor = rec(body.editor)
    const fields = ["name", "user", "host", "port", "key_path"]
    const form: Line[] = fields.map((field) => ({
      label: field,
      detail: str(editor[field]),
    }))
    return {
      title: editor.edit_id ? "remote · edit" : "remote · new",
      hint: body.editing_field ? "enter done editing · esc finish editing" : "tab next field · enter edit · s save · esc back",
      children: (
        <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0}>
          {str(editor.error) ? <Note text={str(editor.error)} theme={theme} color={theme.error} /> : null}
          <FieldList editing={flag(body.editing) || flag(body.editing_field)} rows={form} selected={Math.max(0, fields.indexOf(str(editor.focused)))} theme={theme} onPress={onMove} />
        </box>
      ),
    }
  }
  if (view === "session_hub") {
    const sessions = rows(body.sessions)
    const sessionLines: Line[] = sessions.map((row) => ({
      label: str(row.name) || str(row.session_id),
      detail: flag(row.working) ? "working" : "ready",
    }))
    return {
      title: "remote · sessions",
      hint: "↑↓ · enter open · esc back",
      children: <Lines rows={sessionLines} selected={num(body.session_selected)} theme={theme} onPress={onMove} />,
    }
  }
  const intent = sub.split(":")[0] || "manage"
  const picking = intent === "new" || intent === "resume"
  return {
    title: `remote · ${intent}`,
    hint: picking ? "↑↓ pick · enter connect · esc close" : "↑↓ or j k · enter connect · n new · e edit · d delete · i import · esc close",
    children: managementList("Hosts", theme, lines, selected, onMove, picking ? null : <Action label="+ New (n)" theme={theme} onPress={() => sendKey(send, { Char: "n" })} />),
  }
}

export function extScreenView(body: MenuData["ExtScreen"] & Rec, theme: Theme, onMove: (index: number) => void) {
  const screen = rec(body.screen)
  const title = str(screen.title) || str(body.screen_title) || "extension"
  const nodes = Array.isArray(screen.body) ? screen.body : []
  let menuIndex = 0
  const cursor = num(body.menu_cursor)
  return {
    title,
    hint: "↑↓ · enter select · esc back",
    children: (
      <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0}>
        {str(body.error) ? <Note text={str(body.error)} theme={theme} color={theme.error} /> : null}
        <Document theme={theme} target={`extension-menu-${cursor}`}>{nodes.map((node, index) => {
          const item = rec(node)
          const kind = str(item.t)
          if (kind === "text") return <text key={index} fg={theme.fg}>{str(item.text)}</text>
          if (kind === "kv") {
            return (
              <text key={index}>
                <span fg={theme.dim}>{`${str(item.k).padEnd(16, " ")}`}</span>
                <span fg={theme.fg}>{str(item.v)}</span>
              </text>
            )
          }
          if (kind === "divider") return <text key={index} fg={theme.dim}>────────────────</text>
          if (kind === "menu") {
            const items = rows(item.items).filter((row) => str(row.id))
            const block = items.map((row) => {
              const mine = menuIndex
              menuIndex += 1
              return { label: str(row.label) || str(row.id), index: mine }
            })
            return (
              <box key={index} flexDirection="column">
                {block.map((row) => (
                  <box id={`extension-menu-${row.index}`} key={row.index} height={1} flexShrink={0} paddingLeft={1} backgroundColor={row.index === cursor ? theme.selBg : theme.bg} onMouseDown={() => { if (!body.waiting) onMove(row.index) }}>
                    <text wrapMode="none" fg={row.index === cursor ? theme.selFg : theme.fg}>{row.index === cursor ? `› ${row.label}` : `  ${row.label}`}</text>
                  </box>
                ))}
              </box>
            )
          }
          return null
        })}</Document>
        {str(screen.footer) ? <Note text={str(screen.footer)} theme={theme} /> : null}
      </box>
    ),
  }
}

function pickerView(title: string, picker: Rec, theme: Theme, onMove: (index: number) => void, checks: boolean) {
  const options = Array.isArray(picker.options) ? picker.options : []
  const checked = Array.isArray(picker.checked) ? picker.checked : []
  const lines: Line[] = options.map((row, index) => {
    const label = Array.isArray(row) ? str(row[1] ?? row[0]) : str(row)
    return { label, toggle: checks ? flag(checked[index]) : undefined, toggleLayout: "before" as const, matches: !checks || label.toLowerCase().includes(str(picker.filter).toLowerCase()) }
  }).filter((row)=>row.matches)
  return {
    title,
    hint: checks ? "space toggle · type to filter · enter done · esc cancel" : "↑↓ · enter choose · esc cancel",
    children: (
      <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0}>
        {str(picker.filter) ? <Note text={str(picker.filter)} theme={theme} /> : null}
        <Lines rows={lines} selected={num(picker.cursor)} theme={theme} onPress={onMove} />
      </box>
    ),
  }
}

function draftLines(agent: Rec | null | undefined): string[] {
  if (!agent) return ["no agents"]
  return [
    str(agent.description),
    str(agent.conditions),
    str(agent.model) || "inherit main",
    strings(agent.tools).join(" "),
    clip(str(agent.prompt), 80),
  ].filter(Boolean)
}

export function agentFields(mode: string): string[] {
  const rest = ["description", "conditions", "model", "tools", "prompt"]
  return mode === "create" ? ["name", ...rest] : rest
}

function agentDraft(body: Rec, field: string): string {
  if (field === "name") return str(body.draft_name)
  if (field === "description") return clip(str(body.draft_description), 48)
  if (field === "conditions") return clip(str(body.draft_conditions), 48)
  if (field === "model") return str(body.draft_model_legacy) || str(body.draft_model_uuid) || "inherit"
  if (field === "tools") return clip(str(body.draft_tools), 48)
  return clip(str(body.draft_body), 48)
}

export function mcpFields(transport: string): string[] {
  const head = ["name", "enabled", "transport"]
  return transport === "http" ? [...head, "url"] : [...head, "command", "args", "env"]
}

function mcpDraft(body: Rec, field: string): string {
  if (field === "name") return str(body.draft_name)
  if (field === "transport") return str(body.draft_transport) || "stdio"
  if (field === "url") return str(body.draft_url)
  if (field === "command") return str(body.draft_command)
  if (field === "args") return str(body.draft_args)
  return str(body.draft_env)
}

function managementList(title: string, theme: Theme, lines: Line[], selected: number, onMove: (index: number) => void, actions: ReactNode) {
  return <Panel title={title} hot theme={theme} actions={actions}><Lines rows={lines} selected={selected} theme={theme} onPress={onMove} /></Panel>
}
