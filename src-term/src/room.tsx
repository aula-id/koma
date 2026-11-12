import { useEffect, useRef, useState } from "react"
import { useKeyboard, useTerminalDimensions } from "@opentui/react"
import { BORDER_LEFT, scrollbars, type Theme } from "./theme.ts"
import { Action, Panel } from "./modes/bits.tsx"
import { Choice, promptMark } from "./chat.tsx"
import { suggest } from "./room-suggest.ts"
import { RoomLink, type LaneRow, type ModelChoice, type RoomFrame } from "./room-link.ts"
import { ModelPicker } from "./room-model.tsx"
import { switchDaemon } from "./paths.ts"

export type Line = { id: string; author: string; text: string }
export type ModelList = { choices: ModelChoice[]; lanes: LaneRow[] }
export type Counts = { open: number; verified: number; returned: number; reports: number }

/** The secure room. `/security` again returns to the coding daemon. */
export function RoomScreen({
  id,
  theme,
  onLeave,
}: {
  id: string
  theme: Theme
  onLeave: () => void
}) {
  const [lines, setLines] = useState<Line[]>([])
  const [lanes, setLanes] = useState("")
  const [swim, setSwim] = useState("")
  const [shelf, setShelf] = useState("")
  const [map, setMap] = useState("")
  const [counts, setCounts] = useState<Counts>({ open: 0, verified: 0, returned: 0, reports: 0 })
  const [models, setModels] = useState<ModelList>({ choices: [], lanes: [] })
  const [notice, setNotice] = useState("")
  const [draft, setDraft] = useState("")
  const [phase, setPhase] = useState("starting the secure daemon")
  const link = useRef<RoomLink | null>(null)
  const leaving = useRef(false)

  useEffect(() => {
    let dead = false
    const room = new RoomLink(
      (frame) => { if (!dead) applyFrame(frame, setLines, setLanes, setSwim, setShelf, setMap, setPhase, setCounts, setModels, setNotice) },
      (text) => { if (!dead && !leaving.current) setPhase(text) },
    )
    link.current = room
    void switchDaemon(id, "secure")
      .then(() => room.start(id))
      .catch((error: unknown) => { if (!dead) setPhase(error instanceof Error ? error.message : String(error)) })
    return () => {
      dead = true
      room.close()
    }
  }, [id])

  const send = () => {
    const text = draft.trim()
    setDraft("")
    if (!text) return
    if (text === "/security" || text === "/leave") {
      leaving.current = true
      link.current?.close()
      void switchDaemon(id, "coding").then(onLeave).catch((error: unknown) => {
        leaving.current = false
        setPhase(error instanceof Error ? error.message : String(error))
      })
      return
    }
    setLines((current) => [...current, { id: crypto.randomUUID(), author: "you", text }])
    link.current?.sendUser(text)
  }

  return (
    <RoomView
      theme={theme}
      lines={lines}
      panes={{ lanes, swim, shelf, map }}
      counts={counts}
      phase={phase}
      draft={draft}
      onDraft={setDraft}
      onSend={send}
      models={models}
      notice={notice}
      onOpenModels={() => link.current?.requestModels()}
      onSetModel={(avatar, lane, model) => { setNotice(""); link.current?.setModel(avatar, lane, model) }}
    />
  )
}

export type Panes = { lanes: string; swim: string; shelf: string; map: string }

/** The room, drawn from plain data. The screen above owns the link; this owns the layout. */
export function RoomView({ theme, lines, panes, counts, phase, draft, onDraft, onSend, models = { choices: [], lanes: [] }, notice = "", onOpenModels = () => {}, onSetModel = () => {} }: {
  theme: Theme
  lines: Line[]
  panes: Panes
  counts: Counts
  phase: string
  draft: string
  onDraft: (text: string) => void
  onSend: () => void
  models?: ModelList
  notice?: string
  onOpenModels?: () => void
  onSetModel?: (avatar: string, lane: string, model: string) => void
}) {
  const [full, setFull] = useState<FullView | null>(null)
  const options = suggest(draft)
  const [pickedAt, setPickedAt] = useState(0)
  const picked = Math.min(pickedAt, Math.max(0, options.rows.length - 1))
  const open = options.rows.length > 0
  const accept = (): boolean => {
    const row = options.rows[picked]
    if (!row || row.insert === draft) return false
    onDraft(row.insert)
    return true
  }

  useKeyboard((key) => {
    if (key.eventType === "release") return
    if (full) {
      if (full !== "model" && (key.name === "escape" || key.name === "q")) setFull(null)
      return
    }
    if (open && (key.name === "up" || key.name === "down")) {
      setPickedAt((picked + (key.name === "up" ? -1 : 1) + options.rows.length) % options.rows.length)
    } else if (open && key.name === "tab") accept()
  })

  const submit = () => {
    if (accept()) return
    const view = FULL_COMMANDS[draft.trim()]
    if (view) {
      if (view === "model") onOpenModels()
      setFull(view)
      onDraft("")
      return
    }
    onSend()
  }

  if (full === "model") {
    return <ModelPicker theme={theme} choices={models.choices} lanes={models.lanes} notice={notice} onSet={onSetModel} onBack={() => setFull(null)} />
  }
  if (full) {
    return <FullScreenView view={full} theme={theme} panes={panes} counts={counts} phase={phase} onBack={() => setFull(null)} />
  }

  const ready = phase === "connected"
  return (
    <box flexDirection="column" flexGrow={1} backgroundColor={theme.bg} paddingLeft={2} paddingRight={2} paddingTop={1}>
      <box flexDirection="row" justifyContent="space-between" flexShrink={0}>
        <text flexShrink={0}>
          <span fg={theme.fg}>Koma</span>
          <span fg={theme.dim}> | </span>
          <span fg={theme.accent}>secure room</span>
        </text>
        <text fg={ready ? theme.success : phase === "the room closed" ? theme.warn : theme.dim} wrapMode="none">{ready ? tally(counts, true) : phase}</text>
      </box>
      <scrollbox flexGrow={1} flexBasis={0} minHeight={0} scrollY stickyScroll stickyStart="bottom" focusable={false} scrollbarOptions={scrollbars(theme)}>
        {lines.length === 0 ? (
          <box flexDirection="column" paddingTop={1}>
            <text fg={theme.dim}>Message the Chief to start. The Chief hands work to the specialists.</text>
            <text fg={theme.dim}>@red talks to a specialist directly. /status, /shelf, /trajectory and /model open full screen.</text>
          </box>
        ) : (
          <box flexDirection="column" gap={1} paddingTop={1}>
            {lines.map((line) => <Message key={line.id} line={line} theme={theme} />)}
          </box>
        )}
      </scrollbox>
      {open ? (
        <box height={options.rows.length + 2} flexShrink={0} border borderStyle="rounded" borderColor={theme.dim} flexDirection="column" title={options.title} titleColor={theme.dim}>
          {options.rows.map((row, index) => (
            <Choice key={row.label} label={row.label} detail={row.detail} theme={theme} selected={index === picked} onPress={() => onDraft(row.insert)} />
          ))}
        </box>
      ) : null}
      <box height={3} flexShrink={0}>
        <box border borderStyle="rounded" borderColor={ready ? theme.accent : theme.dim} title=" secure " titleAlignment="right" titleColor={theme.accent} flexGrow={1} flexDirection="row" paddingRight={2}>
          <text fg={theme.accent} flexShrink={0}>{`${promptMark(draft)} `}</text>
          <input
            flexGrow={1}
            value={draft}
            focused
            placeholder="Message the Chief, @red for a specialist, /status for the lanes"
            placeholderColor={theme.dim}
            textColor={theme.fg}
            focusedTextColor={theme.fg}
            backgroundColor={theme.bg}
            focusedBackgroundColor={theme.bg}
            cursorColor={theme.accent}
            onInput={onDraft}
            onSubmit={submit}
          />
        </box>
      </box>
    </box>
  )
}

export type FullView = "status" | "shelf" | "trajectory" | "model"
const FULL_COMMANDS: Record<string, FullView> = { "/status": "status", "/shelf": "shelf", "/trajectory": "trajectory", "/model": "model" }

function FullScreenView({ view, theme, panes, counts, phase, onBack }: { view: Exclude<FullView, "model">; theme: Theme; panes: Panes; counts: Counts; phase: string; onBack: () => void }) {
  const sections: { title: string; text: string; empty: string }[] =
    view === "status"
      ? [
          { title: "Who is working", text: panes.lanes, empty: "No lane is running." },
          { title: "Activity", text: panes.swim, empty: "No lane activity yet." },
        ]
      : view === "shelf"
        ? [{ title: "Findings", text: panes.shelf, empty: "The shelf is empty." }]
        : [{ title: "Trajectory goals", text: panes.map, empty: "No trajectory yet." }]
  return (
    <box flexDirection="column" flexGrow={1} backgroundColor={theme.bg} paddingLeft={2} paddingRight={2} paddingTop={1}>
      <box flexDirection="row" justifyContent="space-between" flexShrink={0}>
        <text flexShrink={0}>
          <span fg={theme.fg}>Koma</span>
          <span fg={theme.dim}> | </span>
          <span fg={theme.accent}>{`secure room · ${view}`}</span>
        </text>
        <Action label="Back (esc)" theme={theme} plain onPress={onBack} />
      </box>
      <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0} paddingTop={1}>
        {sections.map((section, index) => (
          <Panel key={section.title} title={section.title} hot={false} theme={theme} status={view === "status" && index === 0 ? phase === "connected" ? tally(counts, true) : phase : undefined}>
            <scrollbox flexGrow={1} flexBasis={0} minHeight={0} scrollY focusable={false} scrollbarOptions={scrollbars(theme)}>
              {section.text.trim()
                ? section.text.split("\n").map((row, i) => <text key={i} fg={theme.fg} wrapMode="none" paddingLeft={1}>{row || " "}</text>)
                : <text fg={theme.dim} paddingLeft={1}>{section.empty}</text>}
            </scrollbox>
          </Panel>
        ))}
      </box>
    </box>
  )
}

function Message({ line, theme }: { line: Line; theme: Theme }) {
  if (line.author === "you") {
    return (
      <box flexDirection="row" backgroundColor={theme.panel}>
        <box width={1} backgroundColor={theme.accent} />
        <box flexGrow={1} paddingLeft={1} paddingRight={1} paddingTop={1} paddingBottom={1} backgroundColor={theme.panel}>
          <text fg={theme.fg}>{line.text}</text>
        </box>
      </box>
    )
  }
  return (
    <box flexDirection="row">
      <box width={1} border={BORDER_LEFT} borderColor={theme.fg} customBorderChars={DOTTED_RAIL} />
      <box flexGrow={1} flexDirection="column" paddingLeft={1}>
        <text fg={authorInk(line.author, theme)}>{line.author}</text>
        <text fg={theme.fg}>{line.text}</text>
      </box>
    </box>
  )
}

/** Counts of the shelf by gate state; the narrow form keeps what decides the next step. */
export function tally(counts: Counts, full: boolean): string {
  const core = `${counts.open} open · ${counts.verified} verified`
  return full ? `${core} · ${counts.returned} returned · ${counts.reports} reports` : core
}

const DOTTED_RAIL = {
  topLeft: " ", topRight: " ", bottomLeft: " ", bottomRight: " ", horizontal: " ",
  vertical: "┊", topT: " ", bottomT: " ", leftT: " ", rightT: " ", cross: " ",
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
): void {
  const push = (author: string, text: string) => setLines((current) => [...current, { id: crypto.randomUUID(), author, text }])
  switch (frame.type) {
    case "hello":
      setPhase(frame.kind === "secure" ? "connected" : `daemon kind is ${frame.kind}`)
      break
    case "chief":
      push("chief", frame.text)
      break
    case "lane":
      push(frame.avatar, frame.text)
      break
    case "visible":
      push(frame.author, frame.text)
      break
    case "status":
      setLanes(frame.digest)
      setSwim(frame.timeline)
      setShelf(frame.shelf)
      setMap(frame.trajectory)
      setCounts({ open: frame.open, verified: frame.verified, returned: frame.returned, reports: frame.reports })
      break
    case "models":
      setModels({ choices: frame.choices, lanes: frame.lanes })
      break
    case "notice":
      setNotice(frame.text)
      break
    case "closed":
      setPhase("the room closed")
      break
  }
}
