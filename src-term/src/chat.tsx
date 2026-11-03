import { BoxRenderable, CodeRenderable, type RenderNodeContext } from "@opentui/core"
import { syntaxClient } from "./syntax.ts"
import type { Renderable, ScrollBoxRenderable, SyntaxStyle, TextareaRenderable, TextareaAction } from "@opentui/core"
import { useKeyboard, useRenderer } from "@opentui/react"
import { charIndexAt, composerIsAhead, mergeComposerChange, utf16Offset } from "./caret.ts"
import { memo, useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import {
  CHAT_TURNS_DEFAULT,
  CHAT_MOUNT_TURNS,
  CHAT_WINDOW_BYTES,
  countTurns,
  historyRows,
  messageSize,
  nextRenderFrom,
  visibleFromTurn,
  windowEnd,
  shiftWindow,
} from "./history.ts"
import { FolderGuide } from "./dirs.tsx"
import { PALETTE_VISIBLE, commandSuggestions, commandSuggestionOffset, paletteWindow } from "./palette.ts"
import { forwardKey } from "./keys.ts"
import {
  showImagePath,
  type Attachment,
  type ChatMessage,
  type ContextUsage,
  type SessionSnap,
  type Snapshot,
  type ToolCall,
} from "./protocol.ts"
import { SolidScrollBar } from "./modes/bits.tsx"
import { nextStreamPaint, STREAM_PAINT_MS, type StreamPaint } from "./stream-paint.ts"
import { scrollbars, type Theme } from "./theme.ts"
import { Toast, type LocalNotice } from "./toast.tsx"
import { memlogEnabled, registerMemorySource } from "./memlog.ts"
import { pressureNotice, usePressureValve } from "./pressure.ts"

function useTerminalSize() {
  const renderer = useRenderer()
  const [size, setSize] = useState({ width: renderer.width, height: renderer.height })
  useEffect(() => {
    const read = () => setSize((current) => current.width === renderer.width && current.height === renderer.height ? current : { width: renderer.width, height: renderer.height })
    renderer.on("frame", read)
    return () => { renderer.off("frame", read) }
  }, [renderer])
  return size
}

const COMPOSER_KEYS = [
  { name: "return", action: "submit" as const },
  { name: "enter", action: "submit" as const },
  { name: "kpenter", action: "submit" as const },
  { name: "return", shift: true, action: "newline" as const },
  { name: "enter", shift: true, action: "newline" as const },
  { name: "kpenter", shift: true, action: "newline" as const },
  { name: "j", ctrl: true, action: "newline" as const },
]
const PALETTE_KEYS = [
  { name: "up", action: "palette-nav" as const },
  { name: "down", action: "palette-nav" as const },
  { name: "tab", action: "palette-nav" as const },
]

export function Chat({
  snap,
  theme,
  syntaxStyle,
  focused,
  rewind = null,
  overlay = null,
  modalOverlay = null,
  notice = null,
  onNoticeDismiss,
  send,
  sendTracked,
  onFlushError,
}: {
  snap: Snapshot
  theme: Theme
  syntaxStyle: SyntaxStyle
  focused: boolean
  rewind?: { entries: string[]; selected: number } | null
  overlay?: ReactNode
  modalOverlay?: ReactNode
  notice?: LocalNotice | null
  onNoticeDismiss?: () => void
  send: (request: unknown) => void
  sendTracked: (request: unknown) => Promise<string>
  onFlushError: (text: string) => void
}) {
  const flushRef = useRef<() => Promise<boolean>>(async () => true)
  const session = snap.sessions.find((row) => row.id === snap.foreground_id) ?? snap.sessions[0]
  if (!session) {
    return (
      <box flexGrow={1}>
        <text fg={theme.dim}>This daemon has no session yet.</text>
      </box>
    )
  }
  const interaction = modalOverlay ? modalOverlay : session.awaiting_approval ? <Approval session={session} theme={theme} send={send} /> : rewind ? (
    <RewindOverlay rewind={rewind} theme={theme} send={send} />
  ) : overlay ? (
    overlay
  ) : (
    <Palette snap={snap} theme={theme} flushRef={flushRef} send={send} />
  )
  return (
    <box flexDirection="column" flexGrow={1} backgroundColor={theme.bg} paddingTop={1} paddingBottom={1}>
      {snap.global.toast ? <Toast key={`daemon:${snap.global.toast_event_id ?? snap.global.toast.join(":")}`} id={snap.global.toast_event_id ?? snap.global.toast.join(":")} kind={snap.global.toast[0]} text={snap.global.toast[1]} theme={theme} hidden={notice != null} /> : null}
      {notice ? <Toast key={`local:${notice.id}`} id={notice.id} kind={notice.kind ?? "info"} text={notice.text} theme={theme} onDismiss={onNoticeDismiss} /> : null}
      <HistoryPane session={session} theme={theme} syntaxStyle={syntaxStyle} />
      <Steers session={session} theme={theme} selected={snap.global.pending_steer_sel} focused={snap.global.pending_steer_focus} />
      <Composer
        snap={snap}
        session={session}
        theme={theme}
        focused={focused}
        compact={Boolean(modalOverlay)}
        interaction={interaction}
        send={send}
        sendTracked={sendTracked}
        flushRef={flushRef}
        onFlushError={onFlushError}
      />
      <Status snap={snap} session={session} theme={theme} />
    </box>
  )
}

function modeMark(agentMode: string, session: SessionSnap | undefined): { icon: string; label: string; color: string } {
  switch (agentMode) {
    case "auto":
      return { icon: "»", label: "Auto", color: "warn" }
    case "plan":
      return { icon: "●", label: "Planning", color: "info" }
    case "yolo":
      return { icon: "!", label: "Yooloo", color: "error" }
    case "sdlc": {
      const phase = session?.sdlc_phase
      const branch = session?.sdlc_branch
      const label = phase ? (branch ? `sdlc:${phase} · ${branch}` : `sdlc:${phase}`) : "sdlc"
      return { icon: "◆", label, color: "info" }
    }
    default:
      return { icon: "●", label: "Normal", color: "success" }
  }
}

export function HistoryPane({
  session,
  theme,
  syntaxStyle,
}: {
  session: SessionSnap
  theme: Theme
  syntaxStyle: SyntaxStyle
}) {
  const scrollRef = useRef<ScrollBoxRenderable | null>(null)
  const stickRef = useRef(true)
  const expandingRef = useRef(false)
  const restoreRef = useRef<(() => void) | null>(null)
  const sessionRef = useRef(session)
  sessionRef.current = session
  const rows = useMemo(() => historyRows(session.messages), [session.messages])
  const historyKey = `${session.id}:${session.history_rev}`
  const [from, setFrom] = useState(() => visibleFromTurn(historyRows(session.messages), CHAT_TURNS_DEFAULT, CHAT_WINDOW_BYTES))
  const [following, setFollowing] = useState(true)
  const [sticky, setSticky] = useState<ChatMessage | null>(null)
  // Moved during render: a commit with the stale window mounts and drops whole
  // turns, and OpenTUI keeps some native memory for every destroyed renderable.
  const [seen, setSeen] = useState({ rows, historyKey })
  if (seen.rows !== rows || seen.historyKey !== historyKey) {
    const sessionChanged = seen.historyKey !== historyKey
    if (sessionChanged) {
      stickRef.current = true
      setFollowing(true)
      setSticky(null)
    }
    const next = nextRenderFrom(seen.rows, rows, from, stickRef.current, CHAT_TURNS_DEFAULT, sessionChanged, 0, CHAT_WINDOW_BYTES)
    setSeen({ rows, historyKey })
    if (next !== from) setFrom(next)
  }
  const pressureCleanup = useRef(() => {})
  const pressure = usePressureValve(
    session.id,
    () => pressureCleanup.current(),
    (run) => { const box = scrollRef.current; if (box) box.afterLayout(run); else run() },
  )
  const reduced = pressure.reduced
  const mountBytes = reduced ? CHAT_WINDOW_BYTES / 2 : CHAT_WINDOW_BYTES
  const step = reduced ? 15 : 30
  const end = windowEnd(rows, from, reduced ? CHAT_TURNS_DEFAULT : CHAT_MOUNT_TURNS, mountBytes)
  const notice = pressureNotice(pressure)

  useEffect(() => {
    const box = scrollRef.current
    const marker = restoreRef.current
    if (!box || marker == null) return
    restoreRef.current = null
    box.afterLayout(() => {
      marker()
      expandingRef.current = false
    })
  }, [from, pressure.cleanups])

  function expand() {
    if (expandingRef.current || from <= 0) return
    const next = shiftWindow(rows, from, -1, step, mountBytes)
    if (next >= from) return
    const box = scrollRef.current
    if (box) preserveAnchor(box)
    stickRef.current = false
    expandingRef.current = true
    setFollowing(false)
    setFrom(next)
  }

  function preserveAnchor(box: ScrollBoxRenderable) {
    const turns = box.getChildren()[0]?.getChildren() ?? []
    const visible = turns.find((turn) => turn.y + turn.height > box.viewport.y)
    if (visible) restoreRef.current = box.anchor(visible)
  }
  pressureCleanup.current = () => {
    const box = scrollRef.current
    if (!box || stickRef.current) return
    const turn = box.getChildren()[0]?.getChildren().find((node) => node.y + node.height > box.viewport.y)
    const sourceIndex = turn ? turnIndex(turn) : -1
    if (sourceIndex < 0) return
    const next = rows.findIndex((row) => row.sourceIndex === sourceIndex)
    if (next >= 0) { preserveAnchor(box); setFrom(next) }
  }

  function newer() {
    const box = scrollRef.current
    if (!box || expandingRef.current) return
    preserveAnchor(box)
    expandingRef.current = true
    setFrom(shiftWindow(rows, from, 1, step, mountBytes))
  }

  function followTail() {
    const was = stickRef.current
    stickRef.current = true
    setFollowing(true)
    if (was) return
    const tail = visibleFromTurn(rows, CHAT_TURNS_DEFAULT, CHAT_WINDOW_BYTES)
    if (tail !== from) setFrom(tail)
  }

  function updateSticky() {
    const box = scrollRef.current
    const root = box?.getChildren()[0]
    if (!box || !root) return
    const top = box.scrollTop
    let next: ChatMessage | null = null
    for (const turn of root.getChildren()) {
      const index = turnIndex(turn)
      if (index < 0) continue
      const user = turn.getChildren().find((child) => String(child.id ?? "").startsWith("user-"))
      if (!user) continue
      const y = turn.y - root.y
      const pos = y >= 0 && y < box.scrollHeight ? y : turn.y
      if (pos + user.height <= top) {
        next = sessionRef.current.messages[index] ?? null
      } else {
        break
      }
    }
    setSticky((current) => (current?.content === next?.content && current?.role === next?.role ? current : next))
  }

  function syncScroll() {
    const box = scrollRef.current
    if (!box || expandingRef.current) return
    updateSticky()
    const view = box.viewport?.height ?? box.height
    const distance = box.scrollHeight - box.scrollTop - view
    if (distance < 2) {
      if (end < rows.length) { newer(); return }
      followTail()
      return
    }
    if (stickRef.current) {
      stickRef.current = false
      setFollowing(false)
    }
    if (box.scrollTop < 2) expand()
  }

  const start = rows[from]?.sourceIndex ?? rows[visibleFromTurn(rows, CHAT_TURNS_DEFAULT, CHAT_WINDOW_BYTES)]?.sourceIndex ?? 0
  const stop = rows[end]?.sourceIndex ?? session.messages.length
  const mounted = { ...session, messages: session.messages.slice(start, stop),
    ...(end < rows.length ? { streaming: null, stream_reasoning: "", pending_tool_calls: [] } : {}) }
  const memory = useRef({ session, rows, from, end, start, stop, cleanups: pressure.cleanups })
  memory.current = { session, rows, from, end, start, stop, cleanups: pressure.cleanups }
  useEffect(() => {
    if (!memlogEnabled) return
    registerMemorySource(() => {
      const { session, rows, from, end, start, stop, cleanups } = memory.current
      let loadedContentBytes = 0
      for (const message of session.messages) loadedContentBytes += messageSize(message)
      return {
        loadedMessages: session.messages.length,
        mountedMessages: stop - start,
        mountedTurns: countTurns(rows.slice(from, end)),
        loadedContentBytes,
        streaming: Boolean(session.streaming || session.stream_reasoning),
        cleanups,
      }
    })
    return () => registerMemorySource(null)
  }, [])
  useEffect(() => {
    const timer = setInterval(() => updateSticky(), 150)
    return () => clearInterval(timer)
  }, [])

  return (
    <box flexGrow={1} flexBasis={0} minHeight={0} flexDirection="column">
      {sticky ? (
        <box flexShrink={0} backgroundColor={theme.bg}>
          <UserBand message={sticky} theme={theme} syntaxStyle={syntaxStyle} muted />
        </box>
      ) : null}
      <box flexGrow={1} flexBasis={0} minHeight={0}>
        <scrollbox
          ref={scrollRef}
          flexGrow={1}
          stickyScroll={following}
          stickyStart="bottom"
          scrollY
          focusable={false}
          onMouseScroll={() => queueMicrotask(syncScroll)}
          onMouseUp={() => queueMicrotask(syncScroll)}
          scrollbarOptions={scrollbars(theme)}
        >
          <Transcript
            sourceOffset={start}
            session={mounted}
            theme={theme}
            syntaxStyle={syntaxStyle}
          />
        </scrollbox>
        <SolidScrollBar source={scrollRef} theme={theme} />
      </box>
      {!following || notice ? <box height={1} flexShrink={0} flexDirection="row" justifyContent="space-between">
        <text fg={pressure.warned ? theme.warn : theme.dim} flexGrow={1} minWidth={0}>{notice ?? "Reading history"}</text>
        <text fg={theme.accent} onMouseDown={followTail}>Latest ↓</text>
      </box> : null}
    </box>
  )
}

const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
const SHELL_MARK = "\u2064"
const BASH_NUDGE_MARK = "\u2061"
const EXT_PROMPT_MARK = "\u2060\u2060"

type Turn = { at: number; user: ChatMessage | null; rest: { message: ChatMessage; at: number }[] }

/** Group visible messages into turns. `at` is the index in the full session history. */
function groupTurns(messages: ChatMessage[], offset: number): Turn[] {
  const turns: Turn[] = []
  messages.forEach((message, i) => {
    if (message.role === "system" || message.role === "tool") return
    const at = offset + i
    if (message.role === "user") {
      turns.push({ at, user: message, rest: [] })
      return
    }
    if (turns.length === 0) turns.push({ at, user: null, rest: [] })
    turns[turns.length - 1]!.rest.push({ message, at })
  })
  return turns
}

/** History index of a mounted turn box, or -1. */
function turnIndex(node: { id?: unknown }): number {
  const id = String(node.id ?? "")
  return id.startsWith("turn-") ? Number(id.slice("turn-".length)) : -1
}

function Transcript({
  sourceOffset,
  session,
  theme,
  syntaxStyle,
}: {
  sourceOffset: number
  session: SessionSnap
  theme: Theme
  syntaxStyle: SyntaxStyle
}) {
  const results = new Map<string, ChatMessage>()
  for (const message of session.messages) {
    if (message.role === "tool" && message.tool_call_id) results.set(message.tool_call_id, message)
  }
  const used = new Set<string>()
  for (const message of session.messages) {
    if (message.role === "system" || message.role === "tool") continue
    for (const call of message.tool_calls ?? []) used.add(call.id)
  }
  const live = Boolean(session.streaming || session.stream_reasoning)
  const pending = session.pending_tool_calls.filter((call) => !used.has(call.id))
  const turns = groupTurns(session.messages, sourceOffset)
  return (
    <box flexDirection="column" gap={1} paddingLeft={1} paddingRight={1}>
      {turns.map((turn) => (
        <box key={`turn-${session.history_rev}-${turn.at}`} id={`turn-${turn.at}`} flexDirection="column" gap={1}>
          {turn.user ? (
            <box id={`user-${turn.at}`}>
              <HistoryTurn
                message={turn.user}
                theme={theme}
                syntaxStyle={syntaxStyle}
                results={results}
              />
            </box>
          ) : null}
          {turn.rest.map(({ message, at }) => (
            <HistoryTurn
              key={`m-${at}`}
              message={message}
              theme={theme}
              syntaxStyle={syntaxStyle}
              results={results}
            />
          ))}
        </box>
      ))}
      {live || pending.length > 0 ? (
        <AssistantCard theme={theme}>
          {session.stream_reasoning ? (
            <Reasoning text={session.stream_reasoning} theme={theme} syntaxStyle={syntaxStyle} defaultOpen streaming />
          ) : null}
          {session.streaming ? (
            <MarkdownBody content={session.streaming} syntaxStyle={syntaxStyle} fg={theme.fg} bg={theme.bg} streaming />
          ) : null}
          {pending.length > 0 ? (
            <box flexDirection="column" gap={0}>
              {pending.map((call) => (
                <ToolCard key={call.id} call={call} result={results.get(call.id)} theme={theme} syntaxStyle={syntaxStyle} />
              ))}
            </box>
          ) : null}
        </AssistantCard>
      ) : null}
    </box>
  )
}

const HistoryTurn = memo(function HistoryTurn({
  message,
  theme,
  syntaxStyle,
  results,
}: {
  message: ChatMessage
  theme: Theme
  syntaxStyle: SyntaxStyle
  results: Map<string, ChatMessage>
}) {
  if (message.role === "user") {
    if (message.content.startsWith(BASH_NUDGE_MARK) || message.content.startsWith(EXT_PROMPT_MARK)) {
      const body = message.content.replace(BASH_NUDGE_MARK, "").replace(EXT_PROMPT_MARK, "")
      const summary = body.split("\n")[0] ?? ""
      return (
        <text fg={theme.dim} selectable>
          <span fg={theme.success}>✓ </span>
          {summary}
        </text>
      )
    }
    if (message.content.startsWith(SHELL_MARK)) {
      const body = message.content.slice(SHELL_MARK.length)
      const [head, ...rest] = body.split("\n")
      const cmd = (head ?? "").replace(/^\$\s?/, "")
      return (
        <box flexDirection="column">
          <text fg={theme.accent} selectable>
            {`$ ${cmd}`}
          </text>
          {rest.join("\n").trim() ? (
            <text fg={theme.dim} selectable>
              {clip(rest.join("\n"))}
            </text>
          ) : null}
        </box>
      )
    }
    return <UserBand message={message} theme={theme} syntaxStyle={syntaxStyle} />
  }
  return (
    <AssistantCard theme={theme}>
      {message.reasoning ? (
        <Reasoning text={message.reasoning} theme={theme} syntaxStyle={syntaxStyle} defaultOpen={false} />
      ) : null}
      {message.content ? <MarkdownBody content={message.content} syntaxStyle={syntaxStyle} fg={theme.fg} bg={theme.bg} /> : null}
      {message.tool_calls && message.tool_calls.length > 0 ? (
        <box flexDirection="column" gap={0}>
          {message.tool_calls.map((call) => (
            <ToolCard
              key={call.id}
              call={call}
              result={results.get(call.id)}
              theme={theme}
              syntaxStyle={syntaxStyle}
            />
          ))}
        </box>
      ) : null}
    </AssistantCard>
  )
}, (prev, next) => prev.message === next.message && prev.theme === next.theme && prev.syntaxStyle === next.syntaxStyle &&
  (prev.message.tool_calls ?? []).every((call) => prev.results.get(call.id) === next.results.get(call.id)))

function pasteFence(): RegExp {
  return /<<<pasted_text n=(\d+) path="[^"]*">>>([\s\S]*?)<<<end_pasted_text n=\d+>>>/g
}

export function collapsePastedText(content: string): string {
  return content.replace(pasteFence(), (_, n) => `[Pasted Text #${n}]`)
}

export type UserPiece = { kind: "text" | "mark"; text: string }

/** Split a user message so `[Pasted Text #n]` can be drawn as text. Markdown would hide those brackets. */
export function userPieces(content: string): UserPiece[] {
  const source = collapsePastedText(content)
  const pieces: UserPiece[] = []
  const pattern = /\[(?:Pasted Text|Image) #\d+\]/g
  let last = 0
  for (const match of source.matchAll(pattern)) {
    const at = match.index ?? 0
    if (at > last) pieces.push({ kind: "text", text: source.slice(last, at) })
    pieces.push({ kind: "mark", text: match[0] })
    last = at + match[0].length
  }
  if (last < source.length) pieces.push({ kind: "text", text: source.slice(last) })
  return pieces
}

/** Sticky copy of a user message: two lines, then "..." when the rest is longer. */
export function stickyPreview(content: string): string {
  const lines = collapsePastedText(content).replace(/\n+$/, "").split("\n")
  if (lines.length <= 2) return lines.join("\n")
  return `${lines[0]}\n${lines[1]}\n...`
}

/** Queued follow-up, same two-line cap. The attachment count stays on the last visible line. */
export function steerPreview(text: string, attachments: number): string {
  const lines = stickyPreview(text).split("\n")
  if (attachments > 0) {
    let at = lines[lines.length - 1] === "..." ? lines.length - 2 : lines.length - 1
    while (at > 0 && lines[at] === "") at -= 1
    lines[at] = `${lines[at]} · ${attachments} attachments`
  }
  return lines.join("\n")
}

function UserBand({
  message,
  theme,
  syntaxStyle,
  muted = false,
}: {
  message: ChatMessage
  theme: Theme
  syntaxStyle: SyntaxStyle
  muted?: boolean
}) {
  const ink = muted ? theme.dim : theme.accent
  const shown = muted ? stickyPreview(message.content) : message.content
  const pieces = muted ? [] : userPieces(shown)
  return (
    <box flexDirection="column" gap={1}>
      {shown ? (
        <box flexDirection="row" backgroundColor={theme.panel}>
          <box width={1} backgroundColor={ink} />
          <box flexGrow={1} flexDirection="column" paddingLeft={1} paddingRight={1} paddingTop={1} paddingBottom={1} backgroundColor={theme.panel}>
            {muted ? shown.split("\n").map((line, index) => (
              <text key={index} fg={ink}>{line || " "}</text>
            )) : pieces.map((piece, index) => piece.kind === "mark" ? (
              <text key={index} fg={ink}>{piece.text}</text>
            ) : piece.text.trim() ? (
              <MarkdownBody key={index} content={piece.text.trim()} syntaxStyle={syntaxStyle} fg={ink} bg={theme.panel} />
            ) : null)}
          </box>
        </box>
      ) : null}
      {muted ? null : <AttachmentPile attachments={message.attachments ?? []} theme={theme} />}
    </box>
  )
}

const DOTTED_RAIL = {
  topLeft: " ",
  topRight: " ",
  bottomLeft: " ",
  bottomRight: " ",
  horizontal: " ",
  vertical: "┊",
  topT: " ",
  bottomT: " ",
  leftT: " ",
  rightT: " ",
  cross: " ",
}

function AssistantCard({ theme, children }: { theme: Theme; children: ReactNode }) {
  return (
    <box flexDirection="row">
      <box width={1} border={["left"]} borderColor={theme.fg} customBorderChars={DOTTED_RAIL} />
      <box flexGrow={1} flexDirection="column" paddingLeft={1} gap={1}>
        {children}
      </box>
    </box>
  )
}

export function MarkdownBody({
  content,
  syntaxStyle,
  fg,
  bg,
  streaming,
}: {
  content: string
  syntaxStyle: SyntaxStyle
  fg: string
  bg?: string
  streaming?: boolean
}) {
  const live = Boolean(streaming)
  const streamingRef = useRef(live)
  streamingRef.current = live
  // One function for the life of this body. OpenTUI rebuilds a block when this
  // identity changes, which would throw away the in-place code widget.
  const renderNode = useRef((token: { type?: string; lang?: string }, context: RenderNodeContext) =>
    renderMarkdownNode(token, context, streamingRef.current),
  ).current
  const painted = useStreamPaint(content, live)
  return (
    <markdown
      content={painted}
      syntaxStyle={syntaxStyle}
      treeSitterClient={syntaxClient}
      fg={fg}
      bg={bg}
      conceal
      {...{ selectable: true }}
      streaming={streaming}
      internalBlockMode="top-level"
      renderNode={renderNode}
    />
  )
}

/** Paint a live reply a few times a second. A settled string is assigned whole. */
function useStreamPaint(text: string, streaming: boolean): string {
  const [shown, setShown] = useState(text)
  const state = useRef<StreamPaint>({ shown: text, paintedAt: Date.now() })
  const latest = useRef(text)
  latest.current = text
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(() => {
    const clear = () => {
      if (timer.current == null) return
      clearTimeout(timer.current)
      timer.current = null
    }
    if (!streaming) {
      clear()
      state.current = { shown: text, paintedAt: Date.now() }
      setShown(text)
      return
    }
    const decision = nextStreamPaint(state.current, text, Date.now(), true)
    if (decision.shown !== state.current.shown) {
      clear()
      state.current = decision
      setShown(decision.shown)
      return
    }
    if (text === state.current.shown || timer.current != null) return
    const wait = Math.max(0, STREAM_PAINT_MS - (Date.now() - state.current.paintedAt))
    timer.current = setTimeout(() => {
      timer.current = null
      const value = latest.current
      state.current = { shown: value, paintedAt: Date.now() }
      setShown(value)
    }, wait)
  }, [text, streaming])
  useEffect(() => () => {
    if (timer.current != null) clearTimeout(timer.current)
  }, [])
  return streaming ? shown : text
}

type ListNode = {
  width?: number
  content?: unknown
  chunks?: Array<{ text?: string }>
  getChildren?: () => ListNode[]
}

/** Keep lists as real blocks, then give each marker a gutter so wrapped lines hang. */
function renderMarkdownNode(token: { type?: string; lang?: string }, context: RenderNodeContext, streaming: boolean) {
  // A new box around the fence is a different widget, so OpenTUI destroys and
  // reparses the whole fence on every token. The default code widget updates in place.
  if (token.type === "code" && streaming) return null
  if (token.type === "code") {
    const code = context.defaultRender()
    if (!code) return null
    const codeBg = context.syntaxStyle.getStyle("markup.raw")?.bg
    if (code instanceof CodeRenderable) {
      code.conceal = false
      code.drawUnstyledText = true
      code.wrapMode = "word"
      if (codeBg) code.bg = codeBg
    }
    code.marginTop = 0
    code.marginBottom = 0
    const frame = new BoxRenderable(code.ctx, {
      flexDirection: "column", width: "100%", minWidth: 0, flexShrink: 0,
      backgroundColor: codeBg,
      border: true, borderStyle: "single", borderColor: context.syntaxStyle.getStyle("conceal")?.fg ?? "#888888",
      titleColor: context.syntaxStyle.getStyle("default")?.fg,
      title: token.lang ? ` ${token.lang} ` : undefined,
      paddingLeft: 1, paddingRight: 1, paddingTop: 1, paddingBottom: 1,
    })
    frame.add(code)
    return frame
  }
  if (token.type !== "list") return null
  const list = context.defaultRender()
  if (list) dressList(list)
  return list
}

function dressList(list: ListNode) {
  for (const row of list.getChildren?.() ?? []) {
    const cells = row.getChildren?.() ?? []
    const marker = cells[0]
    const text = marker?.chunks?.[0]?.text
    if (!marker || typeof text !== "string") continue
    const trimmed = text.trim()
    if (trimmed === "-" || /^\d+\.$/.test(trimmed)) {
      const width = typeof marker.width === "number" ? marker.width : text.length
      marker.width = width + 2
    }
    const content = cells[1]
    if (!content) continue
    for (const child of content.getChildren?.() ?? []) {
      const nested = child.getChildren?.() ?? []
      const nestedMarker = nested[0]?.getChildren?.()?.[0]?.chunks?.[0]?.text
      if (typeof nestedMarker === "string") dressList(child)
    }
  }
}

function Reasoning({
  text,
  theme,
  syntaxStyle,
  defaultOpen,
  streaming,
}: {
  text: string
  theme: Theme
  syntaxStyle: SyntaxStyle
  defaultOpen: boolean
  streaming?: boolean
}) {
  const [open, setOpen] = useState(defaultOpen)
  useEffect(() => {
    if (defaultOpen) setOpen(true)
  }, [defaultOpen])
  return (
    <box flexDirection="column">
      <box onMouseDown={() => setOpen((value) => !value)}>
        <text fg={theme.dim}>{`${open ? "▾" : "▸"} reasoning`}</text>
      </box>
      {open ? (
        <box border={["left"]} borderColor={theme.dim} paddingLeft={1}>
          <MarkdownBody content={text} syntaxStyle={syntaxStyle} fg={theme.dim} bg={theme.bg} streaming={streaming} />
        </box>
      ) : null}
    </box>
  )
}

export function AttachmentPile({ attachments, theme }: {attachments: Attachment[]; theme: Theme}) {
  if (!attachments.length) return null
  return <box flexDirection="row" flexWrap="wrap" columnGap={1} rowGap={0} minWidth={0}>
    {attachments.map((attachment, index) => <box key={`${attachment.kind}-${attachment.marker_n}-${index}`} maxWidth="100%" border borderStyle="rounded" borderColor={theme.dim} paddingLeft={1} paddingRight={1}>
      <text fg={theme.dim}>{previewLine(attachment.label || `${attachment.kind === "pasted_text" ? "Pasted Text" : "Image"} #${attachment.marker_n}`, 40)}</text>
    </box>)}
  </box>
}

function ToolCard({
  call,
  result,
  theme,
  syntaxStyle,
}: {
  call: ToolCall
  result?: ChatMessage
  theme: Theme
  syntaxStyle: SyntaxStyle
}) {
  const [open, setOpen] = useState(false)
  const [hot, setHot] = useState(false)
  const [tick, setTick] = useState(0)
  const done = Boolean(result)
  useEffect(() => {
    if (done) return
    const timer = setInterval(() => setTick((value) => value + 1), 80)
    return () => clearInterval(timer)
  }, [done])
  const mark = done ? "✓" : (SPINNER[tick % SPINNER.length] ?? "⠋")
  const digest = planDigest(call.function.arguments)
  const image = result ? showImagePath(result.content) : null
  return (
    <box border borderStyle="rounded" borderColor={hot ? theme.accent : theme.dim} flexDirection="column">
      <box
        flexDirection="row"
        justifyContent="space-between"
        paddingLeft={1}
        paddingRight={1}
        onMouseOver={() => setHot(true)}
        onMouseOut={() => setHot(false)}
        onMouseDown={() => setOpen((value) => !value)}
      >
        <text>
          <span fg={done ? theme.accent : theme.info}>{`${mark} `}</span>
          <span fg={theme.fg}>{call.function.name}</span>
          <span fg={theme.dim}>{`  ${toolHint(call.function.arguments)}`}</span>
        </text>
        <text fg={theme.dim}>{open ? "▾" : "▸"}</text>
      </box>
      {open ? (
        <box border={["top"]} borderColor={theme.dim} paddingLeft={1} paddingRight={1} flexDirection="column">
          {digest ? (
            <MarkdownBody content={digest} syntaxStyle={syntaxStyle} fg={theme.fg} bg={theme.bg} />
          ) : (
            <>
              <text fg={theme.dim}>payload</text>
              <text fg={theme.fg} selectable>
                {clip(call.function.arguments || "{}")}
              </text>
            </>
          )}
          {image ? <AttachmentPile attachments={[{kind:"image", marker_n:0, label:"Image attachment", rel_path:image}]} theme={theme} /> : null}
          {result?.content ? (
            <>
              <text fg={theme.dim}>result</text>
              <text fg={theme.fg} selectable>
                {capResult(result.content)}
              </text>
            </>
          ) : null}
        </box>
      ) : null}
    </box>
  )
}

export function Steers({ session, theme, selected, focused }: { session: SessionSnap; theme: Theme; selected: number; focused: boolean }) {
  if (session.pending_steer.length === 0) return null
  return (
    <box border={["top"]} borderColor={theme.dim} paddingLeft={2} paddingRight={2} flexDirection="column">
      {session.pending_steer.map((text, index) => (
        <text key={`${index}-${text.slice(0, 12)}`} fg={focused && index === selected ? theme.accent : theme.dim}>
          {`${focused && index === selected ? "●" : "○"} ${steerPreview(text, session.pending_steer_attachments[index]?.length ?? 0)}`}
        </text>
      ))}
    </box>
  )
}

export function Approval({
  session,
  theme,
  send,
}: {
  session: SessionSnap
  theme: Theme
  send: (request: unknown) => void
}) {
  const call = session.pending_tool_calls[session.tool_idx]
  const name = call?.function.name ?? "tool"
  const plan = name === "plan_ready" || name === "mission_ready"
  const { height, width } = useTerminalSize()
  const detail = plan ? planDigest(call?.function.arguments ?? "") ?? call?.function.arguments ?? "" : payloadPreview(call?.function.arguments ?? "")
  return (
    <box border borderStyle="rounded" borderColor={theme.warn} paddingLeft={1} paddingRight={1} flexDirection="column" flexShrink={0} height={Math.min(12, Math.max(6, height - 11))}>
      <scrollbox flexGrow={1} flexBasis={0} minHeight={0} scrollY focusable={false} scrollbarOptions={scrollbars(theme)}>
        <text fg={theme.warn}>{name}</text>
        {!plan && call ? <text fg={theme.fg}>{toolHint(call.function.arguments)}</text> : null}
        {session.approval_reason ? <text fg={theme.dim}>{session.approval_reason}</text> : null}
        {detail ? <text fg={theme.fg} selectable>{detail}</text> : null}
      </scrollbox>
      <box flexDirection={width < 65 && plan ? "column" : "row"} flexShrink={0}>
        <Choice label="Approve (y)" theme={theme} onPress={() => send(approvalRequest(session, "y"))} />
        {plan ? <Choice label="Approve & compact (a)" theme={theme} onPress={() => send(approvalRequest(session, "a"))} /> : null}
        <Choice label={plan ? "Chat more (n/esc)" : "Deny (n/esc)"} theme={theme} onPress={() => send(approvalRequest(session, "n"))} />
      </box>
    </box>
  )
}

export function approvalRequest(session: SessionSnap, key: string): unknown {
  const call = session.pending_tool_calls[session.tool_idx]
  return call?.function.name === "plan_ready" || call?.function.name === "mission_ready"
    ? { PlanDecision: { decision: key === "y" ? "approve" : key === "a" ? "compact" : "deny" } }
    : { ApproveTool: { approve: key === "y", call_id: call?.id ?? null } }
}

function RewindOverlay({
  rewind,
  theme,
  send,
}: {
  rewind: { entries: string[]; selected: number }
  theme: Theme
  send: (request: unknown) => void
}) {
  const { width } = useTerminalSize()
  const selected = rewind.entries.length === 0 ? 0 : Math.min(rewind.selected, rewind.entries.length - 1)
  const visible = 7
  const [start, setStart] = useState(0)
  useEffect(() => {
    setStart((current) => paletteWindow(current, selected, rewind.entries.length, visible))
  }, [selected, rewind.entries.length])
  const slots = Array.from({ length: visible }, (_, index) => {
    const absolute = start + index
    return absolute < rewind.entries.length ? rewind.entries[absolute] : null
  })
  return (
    <box
      height={10}
      flexShrink={0}
      border
      borderStyle="rounded"
      borderColor={theme.dim}
      title=" edit a previous message "
      titleColor={theme.dim}
      flexDirection="column"
      paddingLeft={1}
      paddingRight={1}
    >
      <box flexGrow={1} flexDirection="column">
        {slots.map((entry, index) => {
          if (entry == null) return <box key={`empty-${index}`} height={1} />
          const absolute = start + index
          const on = absolute === selected
          return (
            <box
              key={`${absolute}-${entry.slice(0, 12)}`}
              height={1}
              backgroundColor={on ? theme.selBg : theme.bg}
              onMouseDown={() => {
                const steps = absolute - selected
                if (steps === 0) return
                const code = steps > 0 ? "Down" : "Up"
                for (let i = 0; i < Math.abs(steps); i++) send({ SendKey: { code, mods: 0 } })
              }}
            >
              <text fg={on ? theme.selFg : theme.fg}>{previewLine(entry, Math.max(8, width - 6))}</text>
            </box>
          )
        })}
      </box>
      <text fg={theme.dim}>↑↓ select · Enter rewind · Esc cancel</text>
    </box>
  )
}

function previewLine(text: string, max: number): string {
  const flat = text.replace(/[\n\t\r]/g, " ")
  const chars = [...flat]
  if (chars.length <= max) return flat
  return `${chars.slice(0, Math.max(0, max - 1)).join("")}…`
}

function Palette({
  snap,
  theme,
  flushRef,
  send,
}: {
  snap: Snapshot
  theme: Theme
  flushRef: { current: () => Promise<boolean> }
  send: (request: unknown) => void
}) {
  const suggestions = commandSuggestions(snap.global.command_palette, snap.global.palette_sel)
  const commands = suggestions.rows
  const files = snap.global.file_palette ?? []
  const command = commands.length > 0
  const rows = command
    ? commands.map((row) => ({ label: row.name, detail: row.desc }))
    : files.map((row) => ({ label: row, detail: "" }))
  const selected = rows.length === 0 ? 0 : command ? suggestions.selected : Math.min(snap.global.palette_sel, rows.length - 1)
  const signature = rows.map((row) => row.label).join("\n")
  const signatureRef = useRef(signature)
  const [start, setStart] = useState(0)
  useEffect(() => {
    const listChanged = signatureRef.current !== signature
    signatureRef.current = signature
    setStart((current) => paletteWindow(listChanged ? 0 : current, selected, rows.length))
  }, [selected, rows.length, signature])
  if (rows.length === 0) return null
  const slots = Array.from({ length: PALETTE_VISIBLE }, (_, index) => rows[start + index] ?? null)
  return (
    <box
      height={PALETTE_VISIBLE + 2}
      flexShrink={0}
      border
      borderStyle="rounded"
      borderColor={theme.dim}
      flexDirection="column"
      title={command ? " commands " : " files "}
      titleColor={theme.dim}
    >
      {slots.map((row, index) => {
        const absolute = start + index
        if (!row) return <box key={`empty-${index}`} height={1} />
        return (
          <Choice
            key={`${row.label}-${absolute}`}
            label={row.label}
            detail={row.detail}
            theme={theme}
            selected={absolute === selected}
            onPress={async () => {
              if (!await flushRef.current()) return
              const steps = command ? commands[absolute]!.sourceIndex - snap.global.palette_sel : absolute - selected
              if (steps === 0) return
              const code = steps > 0 ? "Down" : "Up"
              for (let i = 0; i < Math.abs(steps); i++) send({ SendKey: { code, mods: 0 } })
            }}
          />
        )
      })}
    </box>
  )
}

function Composer({
  snap,
  session,
  theme,
  focused,
  compact = false,
  interaction,
  send,
  sendTracked,
  flushRef,
  onFlushError,
}: {
  snap: Snapshot
  session: SessionSnap
  theme: Theme
  focused: boolean
  compact?: boolean
  interaction?: ReactNode
  send: (request: unknown) => void
  sendTracked: (request: unknown) => Promise<string>
  flushRef: { current: () => Promise<boolean> }
  onFlushError: (text: string) => void
}) {
  const { height, width } = useTerminalSize()
  const renderer = useRenderer()
  const area = useRef<TextareaRenderable | null>(null)
  const lastSent = useRef(snap.global.input)
  const lastCursor = useRef(snap.global.cursor)
  const focusRef = useRef(focused)
  focusRef.current = focused
  const applying = useRef(false)
  const pendingPaste = useRef<{ base: string; done: () => void } | null>(null)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const flushToken = useRef(0)
  const [lines, setLines] = useState(Math.max(1, snap.global.input.split("\n").length))
  const [draft, setDraft] = useState(snap.global.input)
  const [wrapped, setWrapped] = useState(1)
  const innerCap = session.awaiting_approval || compact ? 1 : Math.max(1, Math.floor(height / 2) - 2)
  // Soft-wrapped rows count too, or the textarea scrolls inside a box that never grows.
  const boxHeight = Math.min(innerCap, Math.max(1, lines, wrapped)) + 2
  useEffect(() => {
    const count = Math.max(1, area.current?.virtualLineCount ?? 1)
    setWrapped((current) => current === count ? current : count)
  }, [draft, width])
  const paletteOpen = commandSuggestions(snap.global.command_palette, snap.global.palette_sel).rows.length > 0 || (snap.global.file_palette?.length ?? 0) > 0
  const keyQueue = useRef(Promise.resolve())
  const lastPaletteKey = useRef({ code: "", at: 0 })
  useKeyboard((key) => {
    if (key.eventType === "release" || key.defaultPrevented || !focused) return
    // Shift+Tab is the mode cycle (Auto → Normal → Plan → SDLC). The textarea
    // does not own it, and it must fire even when no slash palette is open.
    if (key.name === "tab" && key.shift) {
      if (typeof key.preventDefault === "function") key.preventDefault()
      keyQueue.current = keyQueue.current.then(async () => {
        if (await flush() && focusRef.current) send({ SendKey: { code: "BackTab", mods: 0 } })
      })
      return
    }
    if (snap.global.pending_steer_focus || !paletteOpen && session.pending_steer.length > 0 && key.name === "up" && area.current?.logicalCursor.row === 0) {
      key.preventDefault()
      const wire = forwardKey(key)
      keyQueue.current = keyQueue.current.then(async () => { if (wire && await flush()) send({ SendKey: wire }) })
      return
    }
    if (key.ctrl && ["b", "p", "r", "v", "x"].includes(key.name) || key.meta && key.name === "e") {
      key.preventDefault()
      keyQueue.current = keyQueue.current.then(async () => {
        const request = { SendKey: { code: { Char: key.name }, mods: key.ctrl ? 2 : 4 } }
        if (key.ctrl && key.name === "v") await daemonEdit(request)
        else if (await flush() && focusRef.current) send(request)
      })
      return
    }
    if (key.name === "escape" && focused) {
      if (typeof key.preventDefault === "function") key.preventDefault()
      keyQueue.current = keyQueue.current.then(async () => {
        const ok = await flush()
        if (!ok) return
        send({ SendKey: { code: "Esc", mods: 0 } })
      })
      return
    }
    if (!paletteOpen) return
    const name = key.name
    if (name !== "up" && name !== "down" && name !== "tab") return
    if (typeof key.preventDefault === "function") key.preventDefault()
    const code = name === "up" ? "Up" : name === "down" ? "Down" : "Tab"
    // One physical key can arrive twice. Drop the duplicate so the highlight
    // moves a single row. Flush first so the daemon already has the / or @ draft.
    const now = Date.now()
    if (code === lastPaletteKey.current.code && now - lastPaletteKey.current.at < 40) return
    lastPaletteKey.current = { code, at: now }
    keyQueue.current = keyQueue.current.then(async () => {
      const ok = await flush()
      if (!ok) return
      if (snap.global.command_palette.length) {
        const offset = commandSuggestionOffset(snap.global.command_palette, snap.global.palette_sel, code === "Up" ? -1 : code === "Down" ? 1 : 0)
        for (let i = 0; i < Math.abs(offset); i++) send({ SendKey: { code: offset < 0 ? "Up" : "Down", mods: 0 } })
        if (code === "Up" || code === "Down") return
      }
      send({ SendKey: { code, mods: 0 } })
    })
  })

  useEffect(() => {
    const read = () => {
      const count = Math.max(1, area.current?.editorView.getTotalVirtualLineCount() ?? 1)
      setLines((previous) => previous === count ? previous : count)
    }
    renderer.on("frame", read)
    return () => { renderer.off("frame", read) }
  }, [renderer])

  useEffect(() => {
    const current = area.current
    if (!current) return
    let text = snap.global.input
    let cursor = snap.global.cursor
    const pasted = pendingPaste.current
    if (pasted) {
      if (text === pasted.base) return
      const merged = mergeComposerChange(pasted.base, current.plainText, text)
      cursor = charIndexAt(merged, utf16Offset(text, cursor) + merged.length - text.length)
      text = merged
      pendingPaste.current = null
      pasted.done()
    }
    const local = current.plainText
    // The echo is an older prefix. Putting it back deletes the letters typed since.
    if (composerIsAhead(local, text)) {
      setDraft(local)
      if (local !== lastSent.current) schedule()
      return
    }
    const localCursor = charIndexAt(local, current.cursorOffset ?? local.length)
    if (text === local && (cursor === localCursor || current.hasSelection())) {
      lastSent.current = snap.global.input
      lastCursor.current = snap.global.cursor
      if (cursor !== localCursor) schedule()
      return
    }
    // Same text, but the caret has already moved past this echo.
    if (text === local && localCursor > cursor) return
    applying.current = true
    if (local !== text) {
      current.setText(text)
      setDraft(text)
      setLines(Math.max(1, text.split("\n").length))
    }
    current.cursorOffset = utf16Offset(text, cursor)
    lastSent.current = snap.global.input
    lastCursor.current = snap.global.cursor
    queueMicrotask(() => {
      applying.current = false
      if (text !== snap.global.input) schedule()
    })
  }, [snap.global.input, snap.global.cursor])

  useEffect(() => {
    return () => {
      if (timer.current) clearTimeout(timer.current)
      focusRef.current = false
      pendingPaste.current?.done()
      pendingPaste.current = null
    }
  }, [])

  async function flush(): Promise<boolean> {
    if (timer.current) {
      clearTimeout(timer.current)
      timer.current = null
    }
    const token = ++flushToken.current
    const current = area.current
    if (!current) return false
    const text = current?.plainText ?? ""
    const cursor = charIndexAt(text, current?.cursorOffset ?? text.length)
    if (text === lastSent.current && cursor === lastCursor.current) return true
    const result = await sendTracked({ ReplaceInput: { text, cursor } })
    if (token !== flushToken.current) return result === "ack"
    if (result !== "ack") {
      onFlushError(result)
      return false
    }
    // A key landed while this request was in flight. Don't record the short text as current.
    const latest = area.current?.plainText ?? text
    if (latest !== text) return true
    lastSent.current = text
    lastCursor.current = cursor
    return true
  }
  flushRef.current = flush

  async function daemonEdit(request: unknown): Promise<void> {
    if (!focusRef.current || !await flush()) return
    while (focusRef.current && area.current && (area.current.plainText !== lastSent.current || charIndexAt(area.current.plainText, area.current.cursorOffset) !== lastCursor.current)) {
      if (!await flush()) return
    }
    if (!focusRef.current) return
    await new Promise<void>((resolve) => {
      const timeout = setTimeout(() => {
        pendingPaste.current = null
        resolve()
        schedule()
      }, 5000)
      pendingPaste.current = { base: area.current?.plainText ?? "", done: () => { clearTimeout(timeout); resolve() } }
      send(request)
    })
  }

  function schedule() {
    if (applying.current || pendingPaste.current) return
    const text = area.current?.plainText ?? ""
    setDraft(text)
    setLines(Math.max(1, text.split("\n").length))
    if (timer.current) clearTimeout(timer.current)
    timer.current = setTimeout(() => {
      timer.current = null
      void flush()
    }, 30)
  }

  const version = snap.global.current_version
  const mode = modeMark(snap.global.agent_mode, session)
  const modeColor = mode.color === "success" ? theme.success : mode.color === "warn" ? theme.warn : mode.color === "error" ? theme.error : theme.info
  return (
    <box flexDirection="column" flexShrink={0}>
      {!session.awaiting_approval && snap.global.pending_attachments.length ? <scrollbox height={3} flexShrink={0} scrollY focusable={false} scrollbarOptions={scrollbars(theme)} contentOptions={{ flexDirection: "row", flexWrap: "wrap" }}>
        {snap.global.pending_attachments.map((attachment) => <box key={`${attachment.kind}-${attachment.marker_n}`} flexDirection="row" border borderStyle="rounded" borderColor={theme.dim}>
          <Choice neutral label={previewLine(attachment.label || `${attachment.kind === "pasted_text" ? "Pasted Text" : "Image"} #${attachment.marker_n}`, Math.max(8, Math.floor((width - 12) / 2)))} theme={theme} onPress={() => {
            if (!focused) return
            keyQueue.current = keyQueue.current.then(async () => {
              if (!await flush()) return
              const marker = `[${attachment.kind === "pasted_text" ? "Pasted Text" : "Image"} #${attachment.marker_n}]`
              const text = area.current?.plainText ?? ""
              const at = text.indexOf(marker)
              if (at < 0) return
              if (area.current) area.current.cursorOffset = at
              if (await flush()) send({ SendKey: { code: { Char: "e" }, mods: 4 } })
            })
          }} />
          <Choice neutral label="×" theme={theme} onPress={() => {
            if (!focused) return
            keyQueue.current = keyQueue.current.then(() => daemonEdit({ RemoveAttachment: { marker_n: attachment.marker_n, kind: attachment.kind ?? "image" } }))
          }} />
        </box>)}
      </scrollbox> : null}
      <box height={1} />
      <box flexDirection="row" justifyContent="space-between" paddingLeft={1} paddingRight={1}>
        <text>
          <span fg={theme.fg}>Koma </span>
          <span fg={theme.dim}>{version}</span>
          {snap.global.latest_version ? <span fg={theme.warn}>{` (new ${snap.global.latest_version} update)`}</span> : null}
          <span fg={theme.dim}> | </span>
          <span fg={modeColor}>{mode.label}</span>
        </text>
        <text fg={theme.dim}>{modelLabel(session.resolved_model_id, session.effort)}</text>
      </box>
      <box height={1} />
      <box flexDirection="column" flexShrink={0}>{interaction}<FolderGuide draft={draft} cwd={session.cwd} theme={theme} /></box>
      <box
        border
        borderStyle="rounded"
        borderColor={focused ? theme.accent : theme.dim}
        title={session.name ? ` ${session.name} ` : undefined}
        titleAlignment="right"
        titleColor={theme.accent}
        height={boxHeight}
        flexDirection="row"
        paddingLeft={0}
        paddingRight={2}
      >
      <text fg={theme.accent}>{`${promptMark(draft)} `}</text>
      <textarea
        ref={area}
        flexGrow={1}
        flexBasis={0}
        minWidth={0}
        focused={focused && !snap.global.pending_steer_focus}
        wrapMode="word"
        {...{ focusable: true }}
        initialValue={snap.global.input}
        textColor={theme.fg}
        focusedTextColor={theme.fg}
        backgroundColor={theme.bg}
        focusedBackgroundColor={theme.bg}
        cursorColor={theme.accent}
        keyBindings={(paletteOpen ? [...COMPOSER_KEYS, ...PALETTE_KEYS] : COMPOSER_KEYS) as { name: string; action: TextareaAction; ctrl?: boolean; shift?: boolean }[]}
        onPaste={(event) => {
          event.preventDefault()
          if (!focused) return
          const text = new TextDecoder().decode(event.bytes)
          area.current?.deleteSelection()
          keyQueue.current = keyQueue.current.then(() => daemonEdit({ Paste: { text } }))
        }}
        onContentChange={schedule}
        onCursorChange={schedule}
        onSubmit={() => {
          if (!focused) return
          keyQueue.current = keyQueue.current.then(async () => {
            const ok = await flush()
            if (ok && focusRef.current) {
              const offset = commandSuggestionOffset(snap.global.command_palette, snap.global.palette_sel, 0)
              for (let i = 0; i < Math.abs(offset); i++) send({ SendKey: { code: offset < 0 ? "Up" : "Down", mods: 0 } })
              send({ SendKey: { code: "Enter", mods: 0 } })
            }
          })
        }}
      />
      </box>
      {compact ? <box height={1} /> : <Tips theme={theme} width={width} />}
    </box>
  )
}

export const COMPOSER_TIPS = [
  "/help lists commands and shortcuts",
  "Shift+Tab cycles Auto, Normal, Plan, and SDLC",
  "Tab completes a slash command or an @ file",
  "Shift+Enter or Ctrl+J starts a new line",
  "Esc stops a running turn",
  "Esc Esc edits a previous message",
  "/usage opens the spend ledger",
  "/notification keeps notices after the toast fades",
  "Ctrl+C asks before quitting",
  "/new starts a session and keeps this one running",
  "/new kill starts a session and closes this one",
  "/resume opens the session hub",
  "/remote adds a host. /new remote only picks one",
  "/mode sets Auto, Normal, Plan, or SDLC",
  "/effort sets how hard the model thinks",
  "/free switches this session to koma-free",
  "/settings changes theme, model, and provider",
  "/model switches the model for a role",
  "/compact summarizes the conversation",
  "/clear clears the chat and keeps the archive",
  "Ctrl+R resends the last message",
  "Ctrl+V pastes an image from the clipboard",
  "Ctrl+E toggles internet simple or full",
  "/bash lists foreground and background jobs",
  "/todo shows the session task list",
  "/skill loads or unloads a skill",
  "/cd changes the session working directory",
  "/update installs the latest koma and reopens this session",
]

/** Model name, plus the selected effort when `/effort` is not the model default. */
export function modelLabel(model: string, effort: string): string {
  const level = effort.trim()
  if (!level) return model
  if (!model) return level
  return `${model} · ${level}`
}

/** Composer glyph. The typed character stays in the draft. */
export function promptMark(input: string): string {
  const head = input[0]
  if (head === "!") return "[>]"
  if (head === "/") return "[/]"
  if (head === "@") return "[@]"
  return "[$]"
}

/** Characters revealed so far, clipped to the row. */
export function tipReveal(tip: string, count: number, width: number): string {
  const room = Math.max(0, width)
  const text = [...tip].slice(0, room).join("")
  return [...text].slice(0, Math.max(0, count)).join("")
}

function Tips({ theme, width }: { theme: Theme; width: number }) {
  const [index, setIndex] = useState(0)
  const [count, setCount] = useState(0)
  const tip = COMPOSER_TIPS[index] ?? ""
  const room = Math.max(0, width - 12)
  const full = [...tip].slice(0, room).join("")
  const visible = tipReveal(tip, count, room)
  const length = [...full].length
  const typing = count < length
  useEffect(() => {
    // Depend on count, not the typing flag. The flag stays true for the whole
    // line, so a timer keyed only to it reveals one character and then stops.
    if (count < length) {
      const timer = setTimeout(() => setCount((value) => value + 1), 18)
      return () => clearTimeout(timer)
    }
    const timer = setTimeout(() => {
      setIndex((value) => (value + 1) % COMPOSER_TIPS.length)
      setCount(0)
    }, 3000)
    return () => clearTimeout(timer)
  }, [count, length, index])
  return (
    <box height={1} flexDirection="row" paddingRight={2}>
      <text width={2} height={1} flexShrink={0} wrapMode="none" fg={typing ? theme.accent : theme.warn}>💡</text>
      <text flexGrow={1} height={1} wrapMode="none">
        <span fg={theme.dim}>Tips: </span>
        <span fg={theme.fg}>{visible}</span>
        {typing ? <span fg={theme.accent}>▍</span> : null}
      </text>
    </box>
  )
}

function Status({ snap, session, theme }: { snap: Snapshot; session: SessionSnap; theme: Theme }) {
  const label = statusLabel(snap.global.status)
  const live = isThinking(snap.global.status, session)
  const elapsed = useWorkElapsed(snap.global.work_elapsed_ms, live)
  const readout = usageReadout(session)
  return (
    <box height={1} flexDirection="row" justifyContent="space-between" paddingLeft={2} paddingRight={2}>
      <text>
        {elapsed == null ? (
          <span fg={theme.dim}>{label}</span>
        ) : (
          [
            <span key="spin" fg={theme.accent}>{`${SPINNER[Math.floor(elapsed / 80) % SPINNER.length] ?? "⠋"} `}</span>,
            <span key="status" fg={theme.fg}>{label}</span>,
            <span key="time" fg={theme.dim}>{`  ${(elapsed / 1000).toFixed(1)}s`}</span>,
          ]
        )}
      </text>
      {readout ? <text fg={theme.accent}>{readout}</text> : null}
    </box>
  )
}

/** Turn failures are already a toast and stay in /notification. */
export function statusLabel(status: string): string {
  if (/^\s*error:/i.test(status)) return "ready"
  return status
}

function isThinking(status: string, session: SessionSnap): boolean {
  if (session.awaiting_approval) return false
  if (status.trim().toLowerCase() === "interrupted") return false
  if (session.working || session.waiting || session.streaming || session.stream_reasoning) return true
  return statusLooksLive(status)
}

/** A live phase is one word, like "thinking". A stop, a slash notice, or an approval stays still. */
export function statusLooksLive(status: string): boolean {
  const word = status.trim().toLowerCase()
  if (!word || word === "ready" || word === "interrupted" || word.includes("approve")) return false
  if (/[\s:<]/.test(word)) return false
  return true
}

function useWorkElapsed(anchor: number | null, running: boolean): number | null {
  const [tick, setTick] = useState(0)
  const started = useRef(Date.now())
  const base = useRef(0)
  useEffect(() => {
    if (!running) return
    started.current = Date.now()
    base.current = anchor ?? 0
  }, [running, anchor])
  useEffect(() => {
    if (!running) return
    const timer = setInterval(() => setTick((value) => value + 1), 80)
    return () => clearInterval(timer)
  }, [running])
  if (!running) return null
  void tick
  return base.current + (Date.now() - started.current)
}

function usageReadout(session: SessionSnap): string | null {
  const usage: ContextUsage | null = session.context_usage
  const input = usage ? usage.prompt_tokens : session.tokens_in
  if (!usage && input === 0 && session.tokens_out === 0 && session.cost === 0) return null
  const context =
    usage && usage.effective_window > 0
      ? `${usage.estimated ? "~" : ""}${Math.round((usage.prompt_tokens / usage.effective_window) * 100)}%`
      : "—%"
  const cached = usage?.cached_tokens ?? session.tokens_cached
  const cache = cached > 0 || usage?.cached_tokens != null ? `[${fmtCount(cached)}]` : ""
  return `${context} ↑${fmtCount(input)}${cache} ↓${fmtCount(session.tokens_out)} $${session.cost.toFixed(4)}`
}

function Choice({
  label,
  detail,
  theme,
  selected,
  neutral = false,
  onPress,
}: {
  neutral?: boolean
  label: string
  detail?: string
  theme: Theme
  selected?: boolean
  onPress: () => void
}) {
  const [hot, setHot] = useState(false)
  const bg = selected ? theme.selBg : hot ? theme.panel : theme.bg
  const labelFg = selected ? theme.selFg : hot ? theme.accent : neutral ? theme.dim : theme.accent
  const detailFg = selected ? theme.selFg : theme.dim
  return (
    <box
      paddingLeft={1}
      paddingRight={1}
      backgroundColor={bg}
      onMouseOver={() => setHot(true)}
      onMouseOut={() => setHot(false)}
      onMouseDown={onPress}
    >
      <text>
        <span fg={labelFg}>{label}</span>
        {detail ? <span fg={detailFg}>{`  ${detail}`}</span> : null}
      </text>
    </box>
  )
}

function charKey(char: string): unknown {
  return { SendKey: { code: { Char: char }, mods: 0 } }
}

function payloadPreview(args: string): string {
  try { return JSON.stringify(JSON.parse(args), null, 2) } catch { return args }
}

function planDigest(args: string): string | null {
  try {
    const parsed = JSON.parse(args) as { highlights?: unknown }
    return typeof parsed.highlights === "string" && parsed.highlights.trim() !== "" ? parsed.highlights : null
  } catch {
    return null
  }
}

function toolHint(args: string): string {
  try {
    const parsed = JSON.parse(args) as Record<string, unknown>
    for (const key of ["path", "file", "command", "cmd", "query", "pattern"]) {
      const value = parsed[key]
      if (typeof value === "string" && value.trim()) return oneLine(value)
    }
    const first = Object.values(parsed).find((value) => typeof value === "string" && value.trim())
    if (typeof first === "string") return oneLine(first)
  } catch {
    // Arguments are not JSON yet; show the raw tail.
  }
  return oneLine(args)
}

function oneLine(text: string): string {
  const line = text.replace(/\s+/g, " ").trim()
  return line.length > 48 ? `${line.slice(0, 48)}…` : line
}

function capResult(text: string): string {
  const lines = text.split("\n")
  if (lines.length <= 15) return text
  return [...lines.slice(0, 14), "...."].join("\n")
}

function clip(text: string): string {
  const lines = text.split("\n")
  if (lines.length <= 8) return text
  return `${lines.slice(0, 8).join("\n")}\n…`
}

function fmtCount(n: number): string {
  if (n < 1000) return String(n)
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}K`
  return `${(n / 1_000_000).toFixed(1)}M`
}
