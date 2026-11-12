import { useEffect, useRef, useState } from "react"
import { scrollbars, type Theme } from "./theme.ts"
import { RoomLink, type RoomFrame } from "./room-link.ts"
import { switchDaemon } from "./paths.ts"

type Line = { id: string; text: string }

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
  const [draft, setDraft] = useState("")
  const [phase, setPhase] = useState("starting the secure daemon")
  const link = useRef<RoomLink | null>(null)
  const leaving = useRef(false)

  useEffect(() => {
    let dead = false
    const room = new RoomLink(
      (frame) => { if (!dead) applyFrame(frame, setLines, setLanes, setSwim, setShelf, setMap, setPhase) },
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
    setLines((current) => [...current, { id: crypto.randomUUID(), text: `you: ${text}` }])
    link.current?.sendUser(text)
  }

  return (
    <box flexDirection="row" flexGrow={1} backgroundColor={theme.bg}>
      <box flexGrow={1} flexDirection="column" paddingLeft={1} paddingRight={1}>
        <text fg={theme.accent}>secure room</text>
        <text fg={theme.dim}>{phase}</text>
        <scrollbox flexGrow={1} scrollY scrollbarOptions={scrollbars(theme)}>
          {lines.map((line) => <text key={line.id} fg={theme.fg}>{line.text}</text>)}
        </scrollbox>
        <input
          value={draft}
          focused
          placeholder="message the Chief, @red for a specialist, or /security to return"
          textColor={theme.fg}
          focusedTextColor={theme.fg}
          backgroundColor={theme.bg}
          focusedBackgroundColor={theme.bg}
          cursorColor={theme.accent}
          onInput={setDraft}
          onSubmit={send}
        />
      </box>
      <box width={36} flexDirection="column" border={["left"]} borderColor={theme.dim} paddingLeft={1}>
        <text fg={theme.accent}>lanes</text>
        <text fg={theme.fg}>{lanes || "—"}</text>
        <text fg={theme.accent}>swim</text>
        <text fg={theme.dim}>{swim || "—"}</text>
        <text fg={theme.accent}>shelf</text>
        <text fg={theme.fg}>{shelf || "—"}</text>
        <text fg={theme.accent}>trajectory</text>
        <text fg={theme.dim}>{map || "—"}</text>
      </box>
    </box>
  )
}

function applyFrame(
  frame: RoomFrame,
  setLines: (update: (current: Line[]) => Line[]) => void,
  setLanes: (text: string) => void,
  setSwim: (text: string) => void,
  setShelf: (text: string) => void,
  setMap: (text: string) => void,
  setPhase: (text: string) => void,
): void {
  const push = (text: string) => setLines((current) => [...current, { id: crypto.randomUUID(), text }])
  switch (frame.type) {
    case "hello":
      setPhase(frame.kind === "secure" ? "connected" : `daemon kind is ${frame.kind}`)
      break
    case "chief":
      push(`chief: ${frame.text}`)
      break
    case "lane":
      push(`${frame.avatar}: ${frame.text}`)
      break
    case "visible":
      push(`${frame.author}: ${frame.text}`)
      break
    case "status":
      setLanes(frame.digest)
      setSwim(frame.timeline)
      setShelf(frame.shelf)
      setMap(frame.trajectory)
      break
    case "closed":
      setPhase("the room closed")
      break
  }
}
