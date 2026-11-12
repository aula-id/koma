import { useEffect, useRef, useState } from "react"
import { useKeyboard, useTerminalDimensions } from "@opentui/react"
import { BORDER_LEFT, scrollbars, type Theme } from "./theme.ts"
import { Action, Panel } from "./modes/bits.tsx"
import { promptMark } from "./chat.tsx"
import { RoomLink, type RoomFrame } from "./room-link.ts"
import { switchDaemon } from "./paths.ts"

export type Line = { id: string; author: string; text: string }
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
  const [draft, setDraft] = useState("")
  const [phase, setPhase] = useState("starting the secure daemon")
  const link = useRef<RoomLink | null>(null)
  const leaving = useRef(false)

  useEffect(() => {
    let dead = false
    const room = new RoomLink(
      (frame) => { if (!dead) applyFrame(frame, setLines, setLanes, setSwim, setShelf, setMap, setPhase, setCounts) },
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
    />
  )
}

export type Panes = { lanes: string; swim: string; shelf: string; map: string }

/** The room, drawn from plain data. The screen above owns the link; this owns the layout. */
export function RoomView({ theme, lines, panes, counts, phase, draft, onDraft, onSend }: {
  theme: Theme
  lines: Line[]
  panes: Panes
  counts: Counts
  phase: string
  draft: string
  onDraft: (text: string) => void
  onSend: () => void
}) {
  const { lanes, swim, shelf, map } = panes
  const setDraft = onDraft
  const send = onSend
  const { width } = useTerminalDimensions()
  const wide = width >= 100
  const [tab, setTab] = useState<Tab | null>(null)
  // Wide screens keep the chat beside one side pane; narrow ones show one pane at a time.
  const active: Tab = wide ? (tab && tab !== "chat" ? tab : "lanes") : tab ?? "chat"

  useKeyboard((key) => {
    if (key.eventType === "release" || key.name !== "tab") return
    const order = wide ? TABS.filter((t) => t !== "chat") : TABS
    setTab(order[(order.indexOf(active) + 1) % order.length] ?? order[0])
  })

  const pick = (next: Tab) => setTab(next)
  const body: Record<Exclude<Tab, "chat">, { text: string; empty: string }> = {
    lanes: { text: lanes, empty: "No lane is running. Ask the Chief to start work." },
    swim: { text: swim, empty: "Lane activity shows up here once work starts." },
    shelf: { text: shelf, empty: "The shelf is empty. Findings land here as lanes write them." },
    map: { text: map, empty: "No trajectory yet." },
  }
  const side = active === "chat" ? null : body[active]
  const ready = phase === "connected"

  const transcript = (
    <scrollbox flexGrow={1} flexBasis={0} minHeight={0} scrollY stickyScroll stickyStart="bottom" focusable={false} scrollbarOptions={scrollbars(theme)}>
      {lines.length === 0 ? (
        <box flexDirection="column" paddingTop={1}>
          <text fg={theme.dim}>Message the Chief to start. The Chief hands work to the specialists.</text>
          <text fg={theme.dim}>@red talks to a specialist directly. /security returns to the coding chat.</text>
        </box>
      ) : (
        <box flexDirection="column" gap={1} paddingTop={1}>
          {lines.map((line) => <Message key={line.id} line={line} theme={theme} />)}
        </box>
      )}
    </scrollbox>
  )

  const composer = (
    <box height={3} flexShrink={0}>
      <box
        border
        borderStyle="rounded"
        borderColor={ready ? theme.accent : theme.dim}
        title=" secure "
        titleAlignment="right"
        titleColor={theme.accent}
        flexGrow={1}
        flexDirection="row"
        paddingRight={2}
      >
        <text fg={theme.accent} flexShrink={0}>{`${promptMark(draft)} `}</text>
        <input
          flexGrow={1}
          value={draft}
          focused
          placeholder={wide ? "Message the Chief, @red for a specialist, /security to leave" : "Message the Chief, or @red"}
          placeholderColor={theme.dim}
          textColor={theme.fg}
          focusedTextColor={theme.fg}
          backgroundColor={theme.bg}
          focusedBackgroundColor={theme.bg}
          cursorColor={theme.accent}
          onInput={setDraft}
          onSubmit={send}
        />
      </box>
    </box>
  )

  const tabs = (wide ? TABS.filter((t) => t !== "chat") : TABS).map((t) => (
    <Action key={t} label={TAB_LABEL[t]} theme={theme} plain primary={t === active} onPress={() => pick(t)} />
  ))

  return (
    <box flexDirection="column" flexGrow={1} backgroundColor={theme.bg} paddingLeft={2} paddingRight={2} paddingTop={1}>
      <box flexDirection="row" justifyContent="space-between" flexShrink={0}>
        <text flexShrink={0}>
          <span fg={theme.fg}>Koma</span>
          <span fg={theme.dim}> | </span>
          <span fg={theme.accent}>secure room</span>
        </text>
        <text fg={ready ? theme.success : phase === "the room closed" ? theme.warn : theme.dim} wrapMode="none">{ready ? tally(counts, wide) : phase}</text>
      </box>
      <box flexDirection="row" flexGrow={1} flexBasis={0} minHeight={0} columnGap={2}>
        {active === "chat" || wide ? (
          <box flexDirection="column" flexGrow={1} flexBasis={0} minWidth={0}>
            {transcript}
          </box>
        ) : null}
        {side ? (
          <box flexDirection="column" width={wide ? 44 : undefined} flexGrow={wide ? 0 : 1} flexShrink={0} flexBasis={wide ? undefined : 0} minWidth={0} paddingTop={1}>
            <box flexDirection="row" flexShrink={0} columnGap={1}>{tabs}</box>
            <Panel title={PANE_TITLE[active]} hot={false} theme={theme}>
              <scrollbox flexGrow={1} flexBasis={0} minHeight={0} scrollY focusable={false} scrollbarOptions={scrollbars(theme)}>
                {side.text.trim()
                  ? side.text.split("\n").map((row, index) => <text key={index} fg={theme.fg} wrapMode="none" paddingLeft={1}>{row || " "}</text>)
                  : <text fg={theme.dim} paddingLeft={1}>{side.empty}</text>}
              </scrollbox>
            </Panel>
          </box>
        ) : null}
      </box>
      {!wide && active === "chat" ? <box flexDirection="row" flexShrink={0} columnGap={1}>{tabs}</box> : null}
      {composer}
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

type Tab = "chat" | "lanes" | "swim" | "shelf" | "map"
const TABS: Tab[] = ["chat", "lanes", "swim", "shelf", "map"]
const TAB_LABEL: Record<Tab, string> = {
  chat: "Chat",
  lanes: "Lanes",
  swim: "Swim",
  shelf: "Shelf",
  map: "Trajectory",
}

const PANE_TITLE: Record<Tab, string> = {
  chat: "Chat",
  lanes: "Who is working",
  swim: "Activity",
  shelf: "Findings",
  map: "Trajectory goals",
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
    case "closed":
      setPhase("the room closed")
      break
  }
}
