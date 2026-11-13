import { memo, useEffect, useMemo, useRef, useState } from "react"
import { SyntaxStyle } from "@opentui/core"
import { useKeyboard, useRenderer, useTerminalDimensions } from "@opentui/react"
import { scrollbars, type Theme } from "./theme.ts"
import { Action, Panel } from "./modes/bits.tsx"
import { AssistantCard, MarkdownBody, Reasoning, SuggestPopup, ToolCard } from "./chat.tsx"
import { RoomComposer } from "./room-composer.tsx"
import { CLOSED, chosen, completion, dismissed, moved, popupView, type PopupState } from "./room-popup.ts"
import { syntaxStyles } from "./syntax.ts"
import { capLines, mountedFrom, reduceLines, say, type Item, type Line } from "./room-turns.ts"
import { RoomLink, type EventInfo, type LaneInfo, type LaneRow, type ModelChoice, type RoomFrame } from "./room-link.ts"
import { busySummary, eventCells, laneCells, laneHeader, phaseMark, standing } from "./room-status.ts"
import { classify } from "./room-suggest.ts"
import { ModelPicker } from "./room-model.tsx"
import { BrowseScreen, type BrowseData, type OpenItem } from "./room-browse.tsx"
import { switchDaemon } from "./paths.ts"
import { prettyText, toolDigest } from "./room-pretty.ts"
import { OpeningSplash } from "./loading.tsx"
import { IDLE_COLLECT_MS, ramTone, useIdleCollect, usePressureValve } from "./pressure.ts"
import { memoryFootprint } from "./footprint.ts"

export type { Line } from "./room-turns.ts"
export type ModelList = { choices: ModelChoice[]; lanes: LaneRow[] }
export type Counts = { open: number; verified: number; returned: number; reports: number }

/** The secure room. `/security` again returns to the coding daemon. */
const DOUBLE_ESC_MS = 600

export function RoomScreen({
  id,
  theme,
  syntaxStyle,
  onLeave,
  ctrlC,
}: {
  id: string
  theme: Theme
  syntaxStyle?: SyntaxStyle
  onLeave: () => void
  ctrlC?: { current: () => void }
}) {
  const [lines, setLines] = useState<Line[]>([])
  const [lanes, setLanes] = useState("")
  const [swim, setSwim] = useState("")
  const [shelf, setShelf] = useState("")
  const [map, setMap] = useState("")
  const [roster, setRoster] = useState<Roster>({ table: [], events: [], nowMs: 0 })
  const [counts, setCounts] = useState<Counts>({ open: 0, verified: 0, returned: 0, reports: 0 })
  const [models, setModels] = useState<ModelList>({ choices: [], lanes: [] })
  const [notice, setNotice] = useState("")
  const [flash, setFlash] = useState("")
  const [inserted, setInserted] = useState<Inserted | null>(null)
  const [browse, setBrowse] = useState<BrowseData | null>(null)
  const [item, setItem] = useState<OpenItem | null>(null)
  const [asking, setAsking] = useState(false)
  const [draft, setDraft] = useState("")
  const [phase, setPhase] = useState("starting the secure daemon")
  // The loading screen covers the wait on both sides: until the room answers, and from the
  // moment `/leave` is typed until the coding daemon is back.
  const [ready, setReady] = useState(false)
  const [failed, setFailed] = useState(false)
  const [going, setGoing] = useState(false)
  useEffect(() => {
    if (!flash) return
    const timer = setTimeout(() => setFlash(""), 5000)
    return () => clearTimeout(timer)
  }, [flash])
  const link = useRef<RoomLink | null>(null)
  const leaving = useRef(false)

  useEffect(() => {
    let dead = false
    const room = new RoomLink(
      (frame) => { if (!dead) applyFrame(frame, setLines, setLanes, setSwim, setShelf, setMap, setPhase, setCounts, setModels, setNotice, setRoster, setFlash, setInserted, setBrowse, setItem) },
      (text) => { if (!dead && !leaving.current) setPhase(text) },
    )
    link.current = room
    void switchDaemon(id, "secure")
      .then(() => room.start(id))
      .then(() => { if (!dead) setReady(true) })
      .catch((error: unknown) => { if (!dead) { setFailed(true); setPhase(error instanceof Error ? error.message : String(error)) } })
    return () => {
      dead = true
      room.close()
    }
  }, [id])

  const leave = () => {
    leaving.current = true
    setAsking(false)
    setGoing(true)
    link.current?.close()
    void switchDaemon(id, "coding").then(onLeave).catch((error: unknown) => {
      leaving.current = false
      setGoing(false)
      setFailed(true)
      setPhase(error instanceof Error ? error.message : String(error))
    })
  }

  // Ctrl+C asks first; a second Ctrl+C while asking confirms. The session screen's own
  // handler comes back when the room closes.
  const askingRef = useRef(false)
  askingRef.current = asking
  const leaveRef = useRef(leave)
  leaveRef.current = leave
  // On the loading screen there is no prompt to answer: Ctrl+C or Esc leaves at once.
  const waitingRef = useRef(false)
  waitingRef.current = !ready && !failed && !going
  useKeyboard((key) => { if (waitingRef.current && key.name === "escape") leaveRef.current() })
  useEffect(() => {
    if (!ctrlC) return
    const before = ctrlC.current
    ctrlC.current = () => { if (askingRef.current || waitingRef.current) leaveRef.current(); else setAsking(true) }
    return () => { ctrlC.current = before }
  }, [ctrlC])

  const send = (override?: string) => {
    const text = (override ?? draft).trim()
    setDraft("")
    if (!text) return
    if (text === "/security" || text === "/leave") {
      leave()
      return
    }
    setLines((current) => capLines([...current, say("you", text)]))
    link.current?.sendUser(text)
  }

  if (going || (!ready && !failed)) return <OpeningSplash theme={theme} />

  return (
    <RoomView
      theme={theme}
      syntaxStyle={syntaxStyle}
      lines={lines}
      panes={{ lanes, swim, shelf, map, ...roster }}
      counts={counts}
      phase={phase}
      draft={draft}
      onDraft={setDraft}
      onSend={send}
      models={models}
      notice={notice}
      flash={flash}
      inserted={inserted}
      onPaste={(text) => link.current?.paste(text)}
      onPasteImage={() => link.current?.pasteImage()}
      onInterrupt={() => link.current?.interrupt()}
      browse={browse}
      item={item}
      onBrowse={() => link.current?.browse()}
      onOpenItem={(key) => link.current?.open(key)}
      onStop={(who) => link.current?.stop(who)}
      asking={asking}
      onAnswer={(yes) => { if (yes) leave(); else setAsking(false) }}
      onOpenModels={() => link.current?.requestModels()}
      onSetModel={(avatar, lane, model) => { setNotice(""); link.current?.setModel(avatar, lane, model) }}
    />
  )
}

/** Text to put at the composer caret. The id tells two identical inserts apart. */
export type Inserted = { id: number; text: string }
let insertSeq = 0

export type Roster = { table: LaneInfo[]; events: EventInfo[]; nowMs: number }
export type Panes = { lanes: string; swim: string; shelf: string; map: string } & Partial<Roster>

/** The room, drawn from plain data. The screen above owns the link; this owns the layout. */
export function RoomView({ theme, syntaxStyle, lines, panes, counts, phase, draft, onDraft, onSend, models = { choices: [], lanes: [] }, notice = "", flash = "", inserted = null, onPaste = () => {}, onPasteImage = () => {}, onInterrupt = () => {}, onStop = () => {}, browse = null, item = null, onBrowse = () => {}, onOpenItem = () => {}, asking = false, onAnswer = () => {}, onOpenModels = () => {}, onSetModel = () => {} }: {
  theme: Theme
  syntaxStyle?: SyntaxStyle
  lines: Line[]
  panes: Panes
  counts: Counts
  phase: string
  draft: string
  onDraft: (text: string) => void
  onSend: (text?: string) => void
  models?: ModelList
  notice?: string
  /** A short message for the composer row (a refused paste). */
  flash?: string
  /** Text the daemon says to put at the caret (a pasted marker). */
  inserted?: Inserted | null
  onPaste?: (text: string) => void
  onPasteImage?: () => void
  /** Esc: stop what the Chief and the specialists are saying. */
  onInterrupt?: () => void
  /** `/stop red`, `/stop all`. */
  onStop?: (who: string) => void
  /** What the shelf, goal and report screens draw, and how they ask for more. */
  browse?: BrowseData | null
  item?: OpenItem | null
  onBrowse?: () => void
  onOpenItem?: (key: string) => void
  /** Ctrl+C was pressed: ask whether to go back to the coding chat. */
  asking?: boolean
  onAnswer?: (yes: boolean) => void
  onOpenModels?: () => void
  onSetModel?: (avatar: string, lane: string, model: string) => void
}) {
  const renderer = useRenderer()
  const ownStyle = useMemo(() => syntaxStyle ?? SyntaxStyle.fromStyles(syntaxStyles(theme), renderer.nativeScene), [syntaxStyle, theme, renderer])
  useEffect(() => () => { if (!syntaxStyle) ownStyle.destroy() }, [syntaxStyle, ownStyle])
  // Same upkeep as the coding chat: a shorter mounted window when RSS runs high, and a sweep
  // once the room is quiet. Every mounted message holds native renderables.
  const pressure = usePressureValve("room", () => {}, (run) => run())
  const working = standing(panes.table ?? []).some((item) => item.spinning)
  useIdleCollect(() => !working, IDLE_COLLECT_MS)
  const shown = mountedFrom(lines, pressure.reduced)
  const [full, setFull] = useState<FullView | null>(null)
  const [popup, setPopup] = useState<PopupState>(CLOSED)
  const options = popupView(draft, popup)
  const lastKey = useRef({ name: "", at: 0 })
  const lastEsc = useRef(0)

  useKeyboard((key) => {
    if (key.eventType === "release") return
    if (asking) {
      if (key.name === "return" || key.name === "enter" || key.name === "y") onAnswer(true)
      else if (key.name === "escape" || key.name === "n") onAnswer(false)
      return
    }
    if (full) {
      if (full === "status" && (key.name === "escape" || key.name === "q")) setFull(null)
      return
    }
    if (key.name === "escape" && !options.open) {
      // One Esc interrupts. A second one right after clears the composer.
      const now = Date.now()
      const twice = now - lastEsc.current < DOUBLE_ESC_MS
      lastEsc.current = twice ? 0 : now
      if (twice) onDraft("")
      else onInterrupt()
      return
    }
    if (!options.open) return
    const name = key.name
    if (name !== "up" && name !== "down" && name !== "tab" && name !== "escape") return
    if (typeof key.preventDefault === "function") key.preventDefault()
    // One physical key can arrive twice. The highlight moves a single row.
    const now = Date.now()
    if (name === lastKey.current.name && now - lastKey.current.at < 40) return
    lastKey.current = { name, at: now }
    if (name === "escape") setPopup(dismissed(draft, options))
    else if (name === "tab") {
      const row = options.rows[options.selected]
      if (row) onDraft(completion(row))
    } else setPopup(moved(popup, draft, options, name === "up" ? -1 : 1))
  })

  const [hint, setHint] = useState("")
  useEffect(() => {
    if (!hint) return
    const timer = setTimeout(() => setHint(""), 4000)
    return () => clearTimeout(timer)
  }, [hint])
  const run = (text: string) => {
    const typed = classify(text)
    if (typed.kind === "stop") {
      onStop(typed.who)
      onDraft("")
      return
    }
    if (typed.kind === "unknown") {
      // A typo is kept in the box to fix, and is not sent as a message.
      setHint(typed.near ? `no command ${typed.word} - did you mean ${typed.near}?` : `no command ${typed.word}`)
      return
    }
    const command = FULL_COMMANDS[text.trim()]
    if (command) {
      if (command === "model") onOpenModels()
      setFull(command)
      onDraft("")
      return
    }
    onSend(text)
  }

  const submit = () => {
    const row = options.open ? options.rows[options.selected] : undefined
    if (row) {
      const pick = chosen(row)
      if ("insert" in pick) onDraft(pick.insert)
      else run(pick.run)
      return
    }
    run(draft)
  }

  const view = asking ? null : full
  if (view === "model") {
    return <ModelPicker theme={theme} choices={models.choices} lanes={models.lanes} notice={notice} onSet={onSetModel} onBack={() => setFull(null)} />
  }
  if (view === "shelf" || view === "trajectory" || view === "report") {
    return <BrowseScreen view={view} theme={theme} syntaxStyle={ownStyle} data={browse} item={item} onOpenItem={onOpenItem} onRefresh={onBrowse} onBack={() => setFull(null)} />
  }
  if (view) {
    return <FullScreenView view={view} theme={theme} panes={panes} counts={counts} phase={phase} onBack={() => setFull(null)} />
  }

  const ready = phase === "connected"
  return (
    <box flexDirection="column" flexGrow={1} backgroundColor={theme.bg} paddingLeft={2} paddingRight={2}>
      <scrollbox flexGrow={1} flexBasis={0} minHeight={0} scrollY stickyScroll stickyStart="bottom" focusable={false} scrollbarOptions={scrollbars(theme)}>
        {lines.length === 0 ? (
          <box flexDirection="column" paddingTop={1}>
            <text fg={theme.dim}>Message the Chief to start. The Chief hands work to the specialists.</text>
            <text fg={theme.dim}>@red talks to a specialist directly. /status, /shelf, /trajectory and /model open full screen.</text>
          </box>
        ) : (
          <box flexDirection="column" gap={1} paddingTop={1}>
            {shown > 0 ? <text fg={theme.dim}>{`${shown} earlier message${shown === 1 ? "" : "s"} not shown`}</text> : null}
            {lines.slice(shown).map((line) => <Message key={line.id} line={line} theme={theme} syntaxStyle={ownStyle} />)}
          </box>
        )}
      </scrollbox>
      {asking ? (
        <box flexShrink={0} border borderStyle="rounded" borderColor={theme.warn} flexDirection="column" paddingLeft={1} paddingRight={1} title=" Quit to chat? " titleColor={theme.warn}>
          <text fg={theme.dim}>Leaves the secure room and returns to the coding chat. The shelf and lanes stay on disk.</text>
          <box flexDirection="row" columnGap={1}>
            <Action label="Quit to chat (enter)" theme={theme} plain primary onPress={() => onAnswer(true)} />
            <Action label="Stay (esc)" theme={theme} plain onPress={() => onAnswer(false)} />
          </box>
        </box>
      ) : null}
      <RoomComposer
        theme={theme}
        draft={draft}
        onDraft={onDraft}
        onSubmit={submit}
        focused={!asking}
        ready={ready}
        inserted={inserted}
        onPaste={onPaste}
        onPasteImage={onPasteImage}
        footer={<box flexDirection="row" justifyContent="space-between"><Standings rows={panes.table ?? []} theme={theme} /><Ram tone={ramTone(pressure)} theme={theme} /></box>}
        status={hint ? { text: hint, color: theme.warn } : flash ? { text: flash, color: theme.warn } : ready ? { text: tally(counts, true), color: theme.success } : { text: phase, color: phase === "the room closed" ? theme.warn : theme.dim }}
        popupOpen={options.open && !asking}
        popup={options.open && !asking ? (
          <SuggestPopup
            rows={options.rows.map((row) => ({ label: row.label, detail: row.detail }))}
            selected={options.selected}
            title={options.title}
            theme={theme}
            onPress={(index) => setPopup({ draft, at: index, dismissed: false })}
          />
        ) : undefined}
        placeholder="Message the Chief, @red for a specialist, /status for the lanes"
      />
    </box>
  )
}

const SPIN = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]

/**
 * Who is on the team, under the composer: each minion's short tag and a mark for its state, with no
 * words. A minion that is thinking shows the braille spinner in sky blue, a ready one a quiet circle,
 * and the rest keep their own shapes and colours (■ stopped, ‖ paused, ✕ failed).
 */
function Standings({ rows, theme }: { rows: LaneInfo[]; theme: Theme }) {
  const items = standing(rows)
  const busy = items.some((item) => item.spinning)
  const [tick, setTick] = useState(0)
  useEffect(() => {
    if (!busy) return
    const timer = setInterval(() => setTick((n) => n + 1), 80)
    return () => clearInterval(timer)
  }, [busy])
  if (!items.length) return <text fg={theme.dim} wrapMode="none">starting the room…</text>
  const ink = { success: theme.success, info: theme.info, warn: theme.warn, error: theme.error, dim: theme.dim }
  return (
    <text wrapMode="none" fg={theme.dim}>
      {items.map((item, i) => (
        <span key={item.name}>
          {i ? "   " : ""}
          <span fg={item.state === "ready" ? theme.dim : theme.fg}>{`${item.alias} `}</span>
          <span fg={ink[item.tone]}>{item.spinning ? SPIN[tick % SPIN.length] : item.glyph}</span>
        </span>
      ))}
    </text>
  )
}

export type FullView = "status" | "shelf" | "trajectory" | "report" | "model"
const FULL_COMMANDS: Record<string, FullView> = { "/status": "status", "/shelf": "shelf", "/trajectory": "trajectory", "/report": "report", "/model": "model" }

function FullScreenView({ view, theme, panes, counts, phase, onBack }: { view: "status"; theme: Theme; panes: Panes; counts: Counts; phase: string; onBack: () => void }) {
  const { width } = useTerminalDimensions()
  const header = (
    <box flexDirection="row" justifyContent="space-between" flexShrink={0}>
      <text flexShrink={0}>
        <span fg={theme.fg}>Koma</span>
        <span fg={theme.dim}> | </span>
        <span fg={theme.accent}>{`secure room · ${view}`}</span>
      </text>
      <Action label="Back (esc)" theme={theme} plain onPress={onBack} />
    </box>
  )
  {
    const table = panes.table ?? []
    const events = panes.events ?? []
    const nowMs = panes.nowMs ?? 0
    const inner = Math.max(40, width - 8)
    return (
      <box flexDirection="column" flexGrow={1} backgroundColor={theme.bg} paddingLeft={2} paddingRight={2} paddingTop={1}>
        {header}
        <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0} paddingTop={1}>
          <Panel title="Lanes" hot={false} theme={theme} maxLines={Math.max(2, table.length + 1)} status={table.length ? busySummary(table) : phase === "connected" ? undefined : phase}>
            {table.length === 0 ? <text fg={theme.dim} paddingLeft={1}>No lane has reported yet.</text> : (
              <box flexDirection="column" paddingLeft={1}>
                <text fg={theme.dim} wrapMode="none">{laneHeader(inner)}</text>
                {table.map((row) => <LaneRowView key={`${row.avatar}-${row.lane}`} row={row} nowMs={nowMs} width={inner} theme={theme} />)}
              </box>
            )}
          </Panel>
          <Panel title="Goals and gates" hot={false} theme={theme} status={tally(counts, true)}>
            <scrollbox flexGrow={1} flexBasis={0} minHeight={0} scrollY focusable={false} scrollbarOptions={scrollbars(theme)}>
              {panes.map.trim()
                ? panes.map.split("\n").map((row, i) => <text key={i} fg={theme.fg} wrapMode="none" paddingLeft={1}>{row || " "}</text>)
                : <text fg={theme.dim} paddingLeft={1}>No goals yet. The Chief sets them when it hands work out.</text>}
            </scrollbox>
          </Panel>
          <Panel title="Activity" hot={false} theme={theme}>
            <scrollbox flexGrow={1} flexBasis={0} minHeight={0} scrollY focusable={false} scrollbarOptions={scrollbars(theme)}>
              {events.length === 0
                ? <text fg={theme.dim} paddingLeft={1}>Nothing has happened yet.</text>
                : events.map((event, i) => {
                    const cells = eventCells(event, nowMs)
                    return (
                      <text key={i} wrapMode="none" paddingLeft={1}>
                        <span fg={theme.dim}>{`${cells.age}  `}</span>
                        <span fg={authorInk(event.avatar, theme)}>{cells.avatar}</span>
                        <span fg={theme.fg}>{` ${cells.marker} ${cells.label}`}</span>
                      </text>
                    )
                  })}
            </scrollbox>
          </Panel>
        </box>
      </box>
    )
  }
}

function LaneRowView({ row, nowMs, width, theme }: { row: LaneInfo; nowMs: number; width: number; theme: Theme }) {
  const cells = laneCells(row, nowMs, width)
  const tone = theme[phaseMark(row.phase).tone]
  const idle = row.phase === "idle"
  return (
    <text wrapMode="none">
      <span fg={tone}>{`${cells.glyph} `}</span>
      <span fg={authorInk(row.avatar, theme)}>{cells.name}</span>
      <span fg={theme.dim}>{` ${cells.lane} `}</span>
      <span fg={tone}>{cells.phase}</span>
      <span fg={idle ? theme.dim : theme.fg}>{` ${cells.task} `}</span>
      <span fg={theme.dim}>{cells.steps}</span>
      {cells.tool ? <span fg={theme.dim}>{` ${cells.tool}`}</span> : null}
      <span fg={theme.dim}>{` ${cells.age}`}</span>
    </text>
  )
}

function Ram({ tone, theme }: { tone: "dim" | "warn" | "error"; theme: Theme }) {
  const [rss, setRss] = useState(() => memoryFootprint())
  useEffect(() => {
    const timer = setInterval(() => setRss(memoryFootprint()), 1000)
    return () => clearInterval(timer)
  }, [])
  const ink = tone === "error" ? theme.error : tone === "warn" ? theme.warn : theme.dim
  return <text fg={ink} wrapMode="none">{`RAM[${Math.round(rss / (1024 * 1024))}mb]`}</text>
}

// A message is replaced, not edited, so an unchanged one keeps its identity and is not drawn again.
const Message = memo(function Message({ line, theme, syntaxStyle }: { line: Line; theme: Theme; syntaxStyle: SyntaxStyle }) {
  if (line.author === "you") {
    const first = line.items[0]
    return (
      <box flexDirection="row" backgroundColor={theme.panel}>
        <box width={1} backgroundColor={theme.accent} />
        <box flexGrow={1} minWidth={0} paddingLeft={1} paddingRight={1} paddingTop={1} paddingBottom={1} backgroundColor={theme.panel}>
          <text fg={theme.fg} wrapMode="word">{first?.kind === "text" ? first.text : ""}</text>
        </box>
      </box>
    )
  }
  const label = line.lane === "work" ? `${line.author} · working` : line.author
  return (
    <AssistantCard theme={theme}>
      <text fg={authorInk(line.author, theme)}>{label}</text>
      {line.items.map((item, index) => <Piece key={index} item={item} theme={theme} syntaxStyle={syntaxStyle} />)}
    </AssistantCard>
  )
})

function Piece({ item, theme, syntaxStyle }: { item: Item; theme: Theme; syntaxStyle: SyntaxStyle }) {
  if (item.kind === "reasoning") return <Reasoning text={item.text} theme={theme} syntaxStyle={syntaxStyle} defaultOpen={Boolean(item.streaming)} streaming={item.streaming} />
  if (item.kind === "tool") {
    const call = { id: item.id, type: "function", function: { name: item.name, arguments: item.args } }
    const result = item.result ? { role: "tool", content: item.result.text, tool_call_id: item.id } : undefined
    return <ToolCard call={call} result={result} theme={theme} syntaxStyle={syntaxStyle} digest={toolDigest(item.name, item.args)} />
  }
  return <MarkdownBody content={item.streaming ? item.text : prettyText(item.text)} syntaxStyle={syntaxStyle} fg={theme.fg} bg={theme.bg} streaming={item.streaming} />
}

/** Counts of the shelf by gate state; the narrow form keeps what decides the next step. */
export function tally(counts: Counts, full: boolean): string {
  const core = `${counts.open} open · ${counts.verified} verified`
  return full ? `${core} · ${counts.returned} returned · ${counts.reports} reports` : core
}

/** The Chief is the accent; specialists take the status colors so lanes read apart at a glance. */
export function authorInk(author: string, theme: Theme): string {
  const name = author.toLowerCase()
  if (name.startsWith("chief")) return theme.accent
  if (name.startsWith("red")) return theme.error
  if (name.startsWith("review")) return theme.info
  if (name.startsWith("report")) return theme.warn
  return theme.success
}

function applyFrame(
  frame: RoomFrame,
  setLines: (update: (current: Line[]) => Line[]) => void,
  setLanes: (text: string) => void,
  setSwim: (text: string) => void,
  setShelf: (text: string) => void,
  setMap: (text: string) => void,
  setPhase: (text: string) => void,
  setCounts: (counts: Counts) => void,
  setModels: (models: ModelList) => void,
  setNotice: (text: string) => void,
  setRoster: (roster: Roster) => void,
  setFlash: (text: string) => void,
  setInserted: (inserted: Inserted) => void,
  setBrowse: (data: BrowseData) => void,
  setItem: (item: OpenItem) => void,
): void {
  switch (frame.type) {
    case "hello":
      setPhase(frame.kind === "secure" ? "connected" : `daemon kind is ${frame.kind}`)
      break
    case "chief":
    case "lane":
    case "visible":
    case "part":
      setLines((current) => capLines(reduceLines(current, frame)))
      break
    case "status":
      setLanes(frame.digest)
      setSwim(frame.timeline)
      setShelf(frame.shelf)
      setMap(frame.trajectory)
      setRoster({ table: frame.lanes, events: frame.events, nowMs: frame.now_ms })
      setCounts({ open: frame.open, verified: frame.verified, returned: frame.returned, reports: frame.reports })
      break
    case "models":
      setModels({ choices: frame.choices, lanes: frame.lanes })
      break
    case "notice":
      setNotice(frame.text)
      setFlash(frame.text)
      break
    case "browse":
      setBrowse({ shelf: frame.shelf, goals: frame.goals, reports: frame.reports })
      break
    case "item":
      setItem({ key: frame.key, revision: frame.revision, updated: frame.updated, content: frame.content })
      break
    case "pasted":
      setInserted({ id: ++insertSeq, text: frame.insert })
      break
    case "closed":
      setPhase("the room closed")
      break
  }
}
