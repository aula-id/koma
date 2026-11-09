import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react"
import type { BoxRenderable, ScrollBoxRenderable, InputRenderable, TextareaRenderable } from "@opentui/core"
import { useKeyboard, useRenderer } from "@opentui/react"
import { composerIsAhead } from "../caret.ts"
import { paletteWindow } from "../palette.ts"
import { scrollbars, type Theme } from "../theme.ts"

const FLUSH_CONTENT = { paddingLeft: 0, paddingRight: 0 } as const

import { select, type ActionDescriptor } from "./actions.ts"

export const InputSuspendedContext = createContext(false)
type NativeDraft = { text: string; cursor: number; selection: {start: number; end: number} | null }
export const NativeDraftContext = createContext<Map<string, NativeDraft> | null>(null)
export const DraftContext = createContext<((field: string, value: string) => void) | null>(null)
export const nativeMenuInput: { current: TextareaRenderable | null; multiline: boolean; listNavigation: boolean; flush?: () => void } = { current: null, multiline: false, listNavigation: false }

const FullscreenContext = createContext(false)

export function Fullscreen({
  title,
  hint,
  theme,
  children,
  contentHeight,
  status,
}: {
  status?: string
  contentHeight?: number
  title: ReactNode
  hint: ReactNode
  theme: Theme
  children: ReactNode
}) {
  return (
    <FullscreenContext.Provider value={true}>
    <box flexGrow={1} minHeight={0} flexDirection="column" backgroundColor={theme.bg} paddingTop={1} paddingBottom={1} gap={1}>
      <box flexShrink={0} minHeight={1}>
        {typeof title === "string" ? <text fg={theme.accent}>{` Koma / ${title}`}</text> : title}
      </box>
      <box flexGrow={1} flexBasis={0} maxHeight={contentHeight} minHeight={0} flexDirection="column">
        {children}
      </box>
      <box flexShrink={0} flexDirection="column">
        {typeof hint === "string" ? <text fg={theme.dim}>{` ${hint}`}</text> : hint}
        {status !== undefined ? <box height={1} flexDirection="row" justifyContent="flex-end"><text fg={theme.info} wrapMode="none">{status}</text></box> : null}
      </box>
    </box>
    </FullscreenContext.Provider>
  )
}

/** Overlay rows, border included. Same budget as the slash palette. */
export const OVERLAY_HEIGHT = 10
/** List rows under the hint, inside that budget. */
export const OVERLAY_BODY = 7

export function Overlay({
  title,
  hint,
  theme,
  children,
  corner,
}: {
  title: ReactNode
  hint: ReactNode
  theme: Theme
  children: ReactNode
  /** Mixed-color label drawn in the top-right border. */
  corner?: ReactNode
}) {
  const frame = (
    <box
      border
      borderStyle="rounded"
      borderColor={theme.dim}
      flexDirection="column"
      height={OVERLAY_HEIGHT}
      flexShrink={0}
      title={typeof title === "string" ? ` ${title} ` : undefined}
      titleColor={theme.dim}
    >
      <box flexGrow={1} flexDirection="column">
        {typeof title !== "string" ? title : null}
        {children}
      </box>
      {typeof hint === "string" ? <text fg={theme.dim}>{` ${hint}`}</text> : hint}
    </box>
  )
  if (!corner) return frame
  return (
    <box height={OVERLAY_HEIGHT} flexShrink={0}>
      {frame}
      <box position="absolute" right={2} top={0} height={1} flexDirection="row" backgroundColor={theme.bg} zIndex={1}>{corner}</box>
    </box>
  )
}

export type Line = { label: string; detail?: string; dim?: boolean; color?: keyof Theme; badge?: { text: string; role: "success" | "warn" | "error" | "info" | "dim" }; toggle?: boolean; toggleLayout?: "before" | "right"; split?: boolean }

/** Title stays on the left. Cut it before it reaches the right-hand info. */
export function fitAside(label: string, aside: string, width: number): string {
  const room = Math.max(1, width - [...aside].length - (aside ? 1 : 0))
  const chars = [...label]
  if (chars.length <= room) return label
  if (room <= 1) return "…"
  return `${chars.slice(0, room - 1).join("")}…`
}

/** Shared on/off glyph for both toggle layouts; focus never changes its state color. */
export function Toggle({ checked, theme }: { checked: boolean; theme: Theme }) {
  return <text width={4} height={1} flexShrink={0} fg={checked ? theme.success : theme.dim} wrapMode="none">{checked ? "[━●]" : "[●─]"}</text>
}

/** Read the actual layout, including subsequent terminal and sibling resizes. */
export function useBodyHeight(fallback = 1) {
  const renderer = useRenderer()
  const ref = useRef<BoxRenderable | null>(null)
  const [size, setSize] = useState({ height: fallback, width: 80 })
  useEffect(() => {
    const node = ref.current
    if (!node) return
    let previous = ""
    const read = () => {
      const height = Math.max(1, Math.floor(node.height))
      const width = Math.max(1, Math.floor(node.width))
      const signature = `${width}:${height}`
      if (signature !== previous) { previous = signature; setSize({ height, width }) }
    }
    // Native-scene boxes reject per-node resize hooks. Frame events run after
    // native layout, and also cover changes to sibling and parent allocations.
    renderer.on("frame", read)
    return () => { renderer.off("frame", read) }
  }, [renderer])
  return { ref, ...size }
}

export function Lines({
  rows,
  selected,
  theme,
  visible,
  rail = false,
  onPress,
}: {
  rows: Line[]
  selected: number
  theme: Theme
  /** Used only until the box reports its real height. */
  visible?: number
  /** Solid thumb when the windowed list is taller than the box. */
  rail?: boolean
  onPress: (index: number) => void
}) {
  const full = useContext(FullscreenContext)
  const { ref: boxRef, height: fitted, width } = useBodyHeight(visible ?? Math.max(1, rows.length))
  const indicator = full && rows.length > fitted && fitted > 2
  const slots = Math.max(1, fitted - (indicator ? 1 : 0))
  const startRef = useRef(0)
  const start = selected < 0 ? 0 : paletteWindow(startRef.current, selected, rows.length, slots)
  startRef.current = start
  const window = rows.slice(start, start + slots)
  const showRail = rail && rows.length > slots
  const thumb = showRail ? Math.max(1, Math.min(slots, Math.round((slots / rows.length) * slots))) : 0
  const railOffset = showRail ? Math.round((start / Math.max(1, rows.length - slots)) * Math.max(0, slots - thumb)) : 0
  return (
    <box ref={boxRef} flexDirection="column" flexGrow={1} flexBasis={0} flexShrink={1} minHeight={0} overflow="hidden">
      {rows.length === 0 ? <text fg={theme.dim}> Nothing here yet</text> : null}
      {window.map((row, index) => {
        const absolute = start + index
        return (
          <LineRow
            key={index}
            row={row}
            on={selected >= 0 && absolute === selected}
            theme={theme}
            full={full}
            width={width}
            onPress={() => onPress(absolute)}
          />
        )
      })}
      {showRail ? (
        <box position="absolute" right={0} top={0} width={1} height={slots} flexDirection="column" zIndex={2} backgroundColor={theme.panel}>
          {Array.from({ length: slots }, (_, index) => {
            const on = index >= railOffset && index < railOffset + thumb
            return <text key={index} height={1} fg={on ? theme.dim : theme.panel} bg={theme.panel}>{on ? "█" : " "}</text>
          })}
        </box>
      ) : null}
      <box position="absolute" left={0} bottom={0} width="100%" height={1} visible={indicator} backgroundColor={theme.bg} zIndex={1}>
        <text fg={theme.dim} wrapMode="none">{` ${start > 0 ? "↑ " : ""}${start + 1}–${Math.min(rows.length, start + slots)} / ${rows.length}${start + slots < rows.length ? " ↓" : ""}`}</text>
      </box>
    </box>
  )
}

function LineRow({
  row,
  on,
  theme,
  onPress,
  full,
  width,
}: {
  row: Line
  on: boolean
  theme: Theme
  onPress: () => void
  full: boolean
  width: number
}) {
  const [hot, setHot] = useState(false)
  const labelColor = on || hot ? theme.accent : row.dim ? theme.dim : theme.fg
  const detailColor = row.color ? theme[row.color] as string : !full && on ? theme.success : theme.dim
  const marker = on ? full ? "› " : "● " : "  "
  const aside = [row.detail, row.badge?.text].filter(Boolean).join("  ")
  const content = <><span fg={labelColor}>{row.split ? fitAside(row.label, aside, Math.max(1, width - 4)) : row.label}</span>
    {row.split ? null : row.badge ? <span fg={theme[row.badge.role]}>{`  ${row.badge.text}`}</span> : null}
    {row.split ? null : row.detail ? <span fg={detailColor}>{`  ${row.detail}`}</span> : null}
  </>
  return (
    <box width="100%" height={1} flexShrink={0} flexDirection={row.toggle != null || row.split ? "row" : "column"} backgroundColor={full ? on || hot ? theme.panel : theme.bg : undefined} paddingLeft={1} paddingRight={1} onMouseOver={() => setHot(true)} onMouseOut={() => setHot(false)} onMouseDown={onPress}>
      {row.toggle != null ? <>
        <text width={2} height={1} flexShrink={0} fg={on ? theme.accent : theme.dim}>{marker}</text>
        {row.toggleLayout === "before" ? <box width={5} height={1} flexShrink={0}><Toggle checked={row.toggle} theme={theme} /></box> : null}
        <box flexGrow={1} flexBasis={0} minWidth={0} overflow="hidden"><text wrapMode="none">{content}</text></box>
        {row.toggleLayout !== "before" ? <box width={5} height={1} flexShrink={0} paddingLeft={1}><Toggle checked={row.toggle} theme={theme} /></box> : null}
      </> : row.split ? <>
        <text width={2} height={1} flexShrink={0} fg={on ? theme.accent : theme.dim}>{marker}</text>
        <box flexGrow={1} flexBasis={0} minWidth={0} overflow="hidden"><text wrapMode="none" fg={labelColor}>{fitAside(row.label, aside, Math.max(1, width - 4))}</text></box>
        {aside ? <text height={1} flexShrink={0} wrapMode="none"><span fg={detailColor}>{row.detail ?? ""}</span>{row.detail && row.badge ? <span fg={theme.dim}>{"  "}</span> : null}{row.badge ? <span fg={theme[row.badge.role]}>{row.badge.text}</span> : null}</text> : null}
      </> : <text wrapMode="none"><span fg={on ? theme.accent : theme.dim}>{marker}</span>{content}</text>}
    </box>
  )
}

export function Note({ text, theme, color }: { text: string; theme: Theme; color?: string }) {
  if (!text) return null
  return <text fg={color ?? theme.dim}>{text}</text>
}

export function FieldList({
  rows,
  selected,
  theme,
  editing,
  onPress,
}: {
  rows: Line[]
  selected: number
  theme: Theme
  editing?: boolean
  onPress: (index: number) => void
}) {
  const full = useContext(FullscreenContext)
  const { ref, height, width } = useBodyHeight(12)
  const start = paletteWindow(0, selected, rows.length, Math.max(1, Math.floor(height / 3)))
  return (
    <box ref={ref} flexDirection="column" flexGrow={1} flexBasis={0} maxHeight={rows.length * 3} minHeight={0} overflow="hidden">
      {rows.slice(start, start + Math.max(1, Math.floor(height / 3))).map((row, offset) => row.toggle != null
        ? <box key={start + offset} height={3} flexShrink={0} paddingTop={1} paddingBottom={1}><LineRow row={row} on={start + offset === selected} theme={theme} full={full} width={width} onPress={() => onPress(start + offset)} /></box>
        : <InputField key={start + offset} field={row.label} label={row.label} value={row.detail ?? ""} active={start + offset === selected} editing={editing && start + offset === selected} theme={theme} onPress={() => onPress(start + offset)} />)}
    </box>
  )
}

export function Split({
  wide,
  theme,
  listTitle,
  list,
  detail,
}: {
  wide: boolean
  theme: Theme
  listTitle?: string
  list: ReactNode
  detail: ReactNode
}) {
  return (
    <box flexDirection={wide ? "row" : "column"} flexGrow={1} flexBasis={0} minHeight={0} gap={1}>
      <box width={wide ? "42%" : "100%"} flexGrow={wide ? 0 : 2} flexBasis={wide ? undefined : 0} minWidth={0} minHeight={0} flexDirection="column">
        <Panel title={listTitle || "select"} hot theme={theme}>
        {list}
        </Panel>
      </box>
      <box flexGrow={1} flexBasis={0} minWidth={0} minHeight={0} flexDirection="column">
        <Panel title="detail" hot={false} theme={theme}>
        {detail}
        </Panel>
      </box>
    </box>
  )
}

export function Panel({
  title,
  mark,
  actions,
  maxLines,
  status,
  hot,
  theme,
  children,
}: {
  title: string
  /** Index of an existing shortcut letter in the title. */
  mark?: number
  /** Clickable actions placed in the top-right border. */
  actions?: ReactNode
  /** Content rows. When set, the box does not grow past this. */
  maxLines?: number
  status?: string
  hot: boolean
  theme: Theme
  children: ReactNode
}) {
  return (
    <box
      flexGrow={maxLines == null ? 1 : 0}
      flexBasis={maxLines == null ? 0 : undefined}
      flexShrink={maxLines == null ? 1 : 0}
      height={maxLines == null ? undefined : maxLines + 2}
      minHeight={0}
      minWidth={0}
      overflow="hidden"
      flexDirection="column"
    >
      <box border borderStyle="rounded" borderColor={hot ? theme.accent : theme.dim} title={` ${title} `} bottomTitle={status ? ` ${status} ` : undefined} bottomTitleAlignment="right" titleColor={mark != null ? hot ? theme.fg : theme.dim : hot ? theme.accent : theme.dim} flexGrow={1} flexBasis={0} minHeight={0} minWidth={0} overflow="hidden" flexDirection="column">
        {children}
      </box>
      {mark != null && mark >= 0 && mark < title.length ? <text position="absolute" left={3 + mark} top={0} height={1} width={1} zIndex={1} fg={theme.accent} bg={theme.bg}>{title[mark]}</text> : null}
      {actions ? <box position="absolute" right={2} top={0} height={1} flexDirection="row" backgroundColor={theme.bg} zIndex={1}>{actions}</box> : null}
    </box>
  )
}

export function sendKey(send: (request: unknown) => void, code: string | { Char: string }) {
  send({ SendKey: { code, mods: 0 } })
}

/** Compatibility for existing view callbacks; direct selection, never arrow replay. */
export function nudge(send: (request: unknown) => void, _from: number, to: number) {
  select(send, to)
}

export function Paragraphs({ lines, theme }: { lines: string[]; theme: Theme }) {
  return (
    <scrollbox flexGrow={1} flexBasis={0} minHeight={0} scrollY focusable={false} scrollbarOptions={scrollbars(theme)} contentOptions={FLUSH_CONTENT}>
      {lines.map((line, index) => (
        <text key={index} fg={theme.fg}>
          {line || " "}
        </text>
      ))}
    </scrollbox>
  )
}

/** Daemon-owned input: the outline makes the editing target visible. */
export function InputField({ label, value, placeholder = "Not set", active = false, editing = active, theme, onPress, field, secret = false, onChange }: {
  label: string; value: string; placeholder?: string; active?: boolean; editing?: boolean; theme: Theme; onPress?: () => void; field?: string; secret?: boolean; onChange?: (value: string) => void
}) {
  const update = useContext(DraftContext)
  const { ref, width } = useBodyHeight(3)
  if ((onChange || update && field) && editing) return <EditableField key={label} label={label} value={value} placeholder={placeholder} theme={theme} secret={secret} listNavigation={["query", "history_query", "Model", "directory"].includes(field ?? "")} syncExternal={field === "directory"} onChange={onChange ?? ((next) => update!(field!, next))} />
  const chars = [...value]
  const capacity = Math.max(1, width - 5)
  const shown = editing && chars.length > capacity ? `…${chars.slice(-(capacity - 1)).join("")}` : value
  return (
    <box ref={ref} height={3} flexShrink={0} border borderStyle="rounded" borderColor={active ? theme.accent : theme.dim} title={` ${label} `} titleColor={active ? theme.accent : theme.dim} flexDirection="column" onMouseDown={onPress}>
      <box height={1} paddingLeft={1} paddingRight={1}>
        <text wrapMode="none"><span fg={value ? theme.fg : theme.dim}>{shown || placeholder}</span>{editing ? <span fg={theme.accent}>▮</span> : null}</text>
      </box>
    </box>
  )
}

export function Action({ label, theme, onPress, primary = false, disabled = false, plain = false }: { label: string; theme: Theme; onPress: () => void; primary?: boolean; disabled?: boolean; plain?: boolean }) {
  const [hot, setHot] = useState(false)
  const full = useContext(FullscreenContext) || plain
  const shortcut = full ? /^(.*?)( \([^()]+\))$/.exec(label) : null
  return <box height={1} flexShrink={0} paddingLeft={1} paddingRight={1} backgroundColor={!full && primary ? theme.selBg : hot ? theme.panel : undefined} onMouseOver={() => setHot(true)} onMouseOut={() => setHot(false)} onMouseDown={() => { if (!disabled) onPress() }}><text fg={disabled ? theme.dim : !full ? primary ? theme.selFg : theme.accent : primary || hot ? theme.accent : theme.fg}>{full ? shortcut ? <><span>{shortcut[1]}</span><span fg={theme.dim}>{shortcut[2]}</span></> : label : `[ ${label} ]`}</text></box>
}

export function Controls({ actions, theme, send }: { actions: ActionDescriptor[]; theme: Theme; send: (request: unknown) => void }) {
  return <box flexDirection="row" flexWrap="wrap" columnGap={1} rowGap={0} flexShrink={0}>
    {actions.map((action, index) => <Action key={index} label={action.label} primary={action.primary} disabled={action.disabled} theme={theme} onPress={() => {
      if (action.action) send({ MenuAction: action.action })
      else if (action.key) send({ SendKey: action.key })
    }} />)}
  </box>
}

function EditableField({ label, value, placeholder, secret, theme, onChange, listNavigation, syncExternal = false }: { label: string; value: string; placeholder: string; secret: boolean; theme: Theme; onChange: (value: string) => void; listNavigation: boolean; syncExternal?: boolean }) {
  const contextDrafts = useContext(NativeDraftContext)
  const localDrafts = useRef(new Map<string, NativeDraft>())
  const drafts = contextDrafts ?? localDrafts.current
  const suspended = useContext(InputSuspendedContext)
  const ref = useRef<InputRenderable | null>(null)
  const saved = drafts.get(label)
  const initial = useRef(saved?.text ?? (secret ? "" : value))
  const [length, setLength] = useState([...initial.current].length)
  const lastSent = useRef(initial.current)
  const remote = useRef(value)
  const acceptRemote = useRef(syncExternal)
  remote.current = value
  acceptRemote.current = syncExternal
  const emit = (text: string) => { if (text !== lastSent.current) { lastSent.current = text; onChange(text) } }
  const remember = () => {
    const node = ref.current
    if (node) drafts.set(label, {text: node.plainText, cursor: node.cursorOffset, selection: node.editorView.getSelection()})
  }
  // Native buffer owns cursor, selection, paste and draft. Snapshot refreshes
  // cannot replace this input while the user is editing it.
  useEffect(() => {
    const node = ref.current
    if (node && saved) {
      node.cursorOffset = saved.cursor
      if (saved.selection) node.setSelection(saved.selection.start, saved.selection.end)
    }
    nativeMenuInput.current = node
    nativeMenuInput.flush = () => {
      if (!node) return
      remember()
      const text = node.plainText
      const next = remote.current
      // Tab already entered this directory. Flushing the old prefix would undo it.
      if (acceptRemote.current && text.length > 0 && next.length > text.length && next.startsWith(text)) return
      emit(text)
    }
    nativeMenuInput.multiline = false
    nativeMenuInput.listNavigation = listNavigation
    return () => { remember(); if (nativeMenuInput.current === node) { nativeMenuInput.current = null; nativeMenuInput.flush = undefined } }
  }, [])
  // Tab writes the entered directory back into the query. Put that path in the
  // box, caret at the end, so the next Tab enters the next directory. A shorter
  // echo of what the user already typed is left alone.
  useEffect(() => {
    if (!syncExternal) return
    const node = ref.current
    if (!node) return
    const local = node.plainText
    if (local === value) {
      lastSent.current = value
      return
    }
    if (composerIsAhead(local, value)) return
    lastSent.current = value
    node.setText(value)
    node.cursorOffset = value.length
    drafts.set(label, { text: value, cursor: value.length, selection: null })
    setLength([...value].length)
  }, [syncExternal, value, label])
  return <box height={3} flexShrink={0} border borderStyle="rounded" borderColor={theme.accent} title={` ${label} `} titleColor={theme.accent} flexDirection="column">
    <box height={1} paddingLeft={1} paddingRight={1}>
      <input ref={ref} flexGrow={1} focused={!suspended} keyBindings={[{ name: "a", ctrl: true, action: "select-all" }]} value={initial.current} placeholder={placeholder} textColor={secret ? theme.bg : theme.fg} focusedTextColor={secret ? theme.bg : theme.fg} backgroundColor={theme.bg} focusedBackgroundColor={theme.bg} cursorColor={theme.accent} onCursorChange={remember} onInput={(next: string) => { remember(); setLength([...next].length); emit(next) }} />
      {secret ? <box position="absolute" left={0} top={0}><text fg={theme.fg}>{"•".repeat(Math.min(60, length))}</text></box> : null}
    </box>
  </box>
}

/** Solid thumb over OpenTUI's half-block slider, which otherwise reads as a dashed rail. */
export function SolidScrollBar({ source, theme }: { source: { current: ScrollBoxRenderable | null }; theme: Theme }) {
  const renderer = useRenderer()
  const [rail, setRail] = useState<{ height: number; thumb: number; offset: number } | null>(null)
  useEffect(() => {
    const read = () => {
      const box = source.current
      if (!box) return
      const view = box.viewport?.height ?? 0
      const size = box.scrollHeight
      const height = box.height
      const top = box.scrollTop
      if (height < 2 || size <= view + 0.5) {
        setRail((current) => current === null ? current : null)
        return
      }
      const thumb = Math.max(1, Math.min(height, Math.round((view / size) * height)))
      const offset = Math.round((top / Math.max(1, size - view)) * Math.max(0, height - thumb))
      setRail((current) => current && current.height === height && current.thumb === thumb && current.offset === offset ? current : { height, thumb, offset })
    }
    renderer.on("frame", read)
    return () => { renderer.off("frame", read) }
  }, [renderer, source])
  if (!rail) return null
  return (
    <box position="absolute" right={0} top={0} width={1} height={rail.height} flexDirection="column" zIndex={3} backgroundColor={theme.panel}>
      {Array.from({ length: rail.height }, (_, index) => {
        const on = index >= rail.offset && index < rail.offset + rail.thumb
        return <text key={index} height={1} fg={on ? theme.dim : theme.panel} bg={theme.panel}>{on ? "█" : " "}</text>
      })}
    </box>
  )
}

export function Document({ children, target, theme }: { children: ReactNode; target?: string; theme?: Theme }) {
  const ref = useRef<ScrollBoxRenderable | null>(null)
  useKeyboard((key) => {
    if (key.eventType === "release" || nativeMenuInput.current) return
    if (!target && key.name === "up") ref.current?.scrollBy(-1)
    if (!target && key.name === "down") ref.current?.scrollBy(1)
    if (key.name === "pageup") ref.current?.scrollBy(-1, "viewport")
    if (key.name === "pagedown") ref.current?.scrollBy(1, "viewport")
    if (!target && key.name === "home") ref.current?.scrollTo(0)
    if (!target && key.name === "end") ref.current?.scrollTo(ref.current.scrollHeight)
  })
  const renderer = useRenderer()
  useEffect(() => {
    if (!target) return
    let previous = ""
    const follow = () => {
      const box = ref.current
      if (!box) return
      const signature = `${box.width}:${box.height}:${box.scrollHeight}`
      if (signature === previous) return
      previous = signature
      box.scrollChildIntoView(target)
    }
    renderer.on("frame", follow)
    return () => { renderer.off("frame", follow) }
  }, [renderer, target])
  return <box flexGrow={1} flexBasis={0} minHeight={0}>
    <scrollbox ref={ref} flexGrow={1} flexBasis={0} minHeight={0} scrollY focusable={false} scrollbarOptions={theme ? scrollbars(theme) : undefined} contentOptions={FLUSH_CONTENT}>{children}</scrollbox>
    {theme ? <SolidScrollBar source={ref} theme={theme} /> : null}
  </box>
}
