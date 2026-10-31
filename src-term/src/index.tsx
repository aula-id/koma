import { writeFileSync } from "node:fs"
import { createCliRenderer, SyntaxStyle, type SyntaxStyle as SyntaxStyleHandle } from "@opentui/core"
import { createRoot, useKeyboard, useRenderer, useSelectionHandler } from "@opentui/react"
import { RemoteScreen } from "./remote-screen.tsx"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useColumns } from "./columns.ts"
import { copyText } from "./copy.ts"
import { Hub } from "./hub.tsx"
import { launchPalette, listSessions, savedPalette } from "./paths.ts"
import { nativeMenuInput } from "./modes/bits.tsx"
import type { RemoteHandoff } from "./protocol.ts"
import { QuitScreen, type QuitState } from "./quit.tsx"
import { SessionScreen } from "./session.tsx"
import { syntaxStyles } from "./syntax.ts"
import { themeFor } from "./theme.ts"
import { startMemoryLog } from "./memlog.ts"

type Route = { kind: "hub" } | { kind: "session"; id: string } | { kind: "remote"; remote: RemoteHandoff }

function initialRoute(): Route {
  if (process.env.KOMA_REMOTE) return { kind: "remote", remote: JSON.parse(process.env.KOMA_REMOTE) as RemoteHandoff }
  if (process.env.KOMA_START === "hub") return { kind: "hub" }
  return { kind: "session", id: process.env.KOMA_SESSION || crypto.randomUUID() }
}

const MIN_COLUMNS = 50
/** Matches `UPDATE_EXIT` in `src-agent/src/app/term_preview.rs`. */
const UPDATE_EXIT = 75
const UPDATE_FILE = process.env.KOMA_UPDATE_FILE

process.title = "koma"
startMemoryLog()

function isCtrlC(key: { name?: string; ctrl?: boolean; sequence?: string }): boolean {
  return (key.ctrl && key.name === "c") || key.sequence === "\u0003" || key.name === "ctrl+c"
}

let onInterrupt = () => {}

function App({ syntaxStyle: baseSyntaxStyle }: { syntaxStyle: SyntaxStyleHandle }) {
  const renderer = useRenderer()
  const [route, setRoute] = useState<Route>(initialRoute)
  const [hubQuit, setHubQuit] = useState<QuitState | null>(null)
  const width = useColumns()
  const tooSmall = width < MIN_COLUMNS
  const routeRef = useRef(route)
  const hubQuitRef = useRef(hubQuit)
  const tooSmallRef = useRef(tooSmall)
  const sessionCtrlC = useRef<() => void>(() => {})
  routeRef.current = route
  hubQuitRef.current = hubQuit
  tooSmallRef.current = tooSmall
  // The saved palette, including while the hub is on screen. The hub paints dark
  // on its own; a session splash must not wait for the daemon snapshot.
  const [palette, setPalette] = useState(() => savedPalette())
  const shown = launchPalette(route.kind, palette)
  const theme = themeFor(shown)
  const syntaxStyle = useMemo(() => shown === "dark" ? baseSyntaxStyle : SyntaxStyle.fromStyles(syntaxStyles(themeFor(shown)), renderer.nativeScene), [shown, renderer, baseSyntaxStyle])
  useEffect(() => () => { if (syntaxStyle !== baseSyntaxStyle) syntaxStyle.destroy() }, [syntaxStyle, baseSyntaxStyle])
  renderer.setBackgroundColor(theme.bg)
  useEffect(() => {
    renderer.setBackgroundColor(theme.bg)
    renderer.setTerminalTitle("koma")
  }, [renderer, theme.bg])
  const returnSession = useRef<string | null>(null)
  const onHome = useCallback(() => setRoute((current) => { if (current.kind === "session") returnSession.current=current.id; return { kind: "hub" } }), [])
  const openHubQuit = () => { void listSessions().then((rows) => setHubQuit({working:rows.live.filter((r)=>r.working).length,total:rows.live.length,selected:2})).catch(()=>setHubQuit({working:0,total:0,selected:2})) }
  const onOpen = useCallback((id: string) => {
    setPalette(savedPalette())
    setRoute({ kind: "session", id })
  }, [])
  const onRemote = useCallback((remote: RemoteHandoff) => {
    setPalette(savedPalette())
    setRoute({ kind: "remote", remote })
  }, [])

  useEffect(() => {
    onInterrupt = () => {
      if (tooSmallRef.current) {
        renderer.destroy()
        syntaxStyle.destroy()
        process.exit(0)
      }
      if (routeRef.current.kind === "hub") {
        if (!hubQuitRef.current) openHubQuit()
        return
      }
      sessionCtrlC.current()
    }
  }, [])

  function leave() {
    renderer.destroy()
    syntaxStyle.destroy()
    process.exit(0)
  }

  function update(id: string) {
    writeFileSync(UPDATE_FILE!, id)
    renderer.destroy()
    syntaxStyle.destroy()
    process.exit(UPDATE_EXIT)
  }

  useSelectionHandler((selection) => {
    const text = selection.getSelectedText()
    if (text) copyText(text)
  })

  useKeyboard((key) => {
    if (tooSmall) {
      if (key.eventType === "release") return
      if (key.name === "q" || key.name === "escape" || key.name === "return" || key.name === "enter" || isCtrlC(key)) leave()
      return
    }
    if (route.kind !== "hub") return
    if (key.eventType === "release") return
    if (isCtrlC(key)) {
      if (!hubQuit) openHubQuit()
      return
    }
    if (hubQuit) {
      if (key.name === "left" || key.name === "h") {
        setHubQuit((current) => current && { ...current, selected: Math.max(0, current.selected - 1) })
      } else if (key.name === "right" || key.name === "l") {
        setHubQuit((current) => current && { ...current, selected: Math.min(2, current.selected + 1) })
      } else if (key.name === "tab") {
        setHubQuit((current) => current && { ...current, selected: (current.selected + (key.shift ? 2 : 1)) % 3 })
      } else if (key.name === "escape") setHubQuit(null)
      else if (key.name === "k" || ((key.name === "return" || key.name === "enter") && hubQuit.selected === 0)) leave()
      else if (key.name === "d" || ((key.name === "return" || key.name === "enter") && hubQuit.selected === 1)) leave()
      else if ((key.name === "return" || key.name === "enter") && hubQuit.selected === 2) setHubQuit(null)
      return
    }
    if (key.name === "q" && !nativeMenuInput.current) leave()
  })

  let screen
  if (tooSmall) {
    screen = <TooNarrow theme={theme} width={width} />
  } else if (route.kind === "remote") {
    screen = <RemoteScreen remote={route.remote} theme={theme} palette={palette} syntaxStyle={syntaxStyle} onHome={onHome} onExit={leave} onRemote={onRemote} onPalette={setPalette} ctrlC={sessionCtrlC} />
  } else if (route.kind === "session") {
    screen = (
      <SessionScreen
        key={route.id}
        id={route.id}
        initialPalette={palette}
        syntaxStyle={syntaxStyle}
        onHome={onHome}
        onOpen={onOpen}
        onExit={leave}
        onRemote={onRemote}
        onUpdate={UPDATE_FILE ? update : undefined}
        onPalette={setPalette}
        ctrlC={sessionCtrlC}
      />
    )
  } else {
    screen = <Hub overlay={hubQuit ? <QuitScreen scope="hub" state={hubQuit} theme={theme} exiting={null} onPick={(index) => {if(index===2)setHubQuit(null);else leave()}} /> : null} theme={theme} onOpen={onOpen} currentId={returnSession.current} onBack={returnSession.current ? () => onOpen(returnSession.current!) : undefined} />
  }
  if (tooSmall) {
    return <TooNarrow theme={theme} width={width} />
  }
  return (
    <box flexGrow={1} flexDirection="column" backgroundColor={theme.bg} paddingLeft={2} paddingRight={2}>
      {screen}
    </box>
  )
}

function TooNarrow({ theme, width }: { theme: ReturnType<typeof themeFor>; width: number }) {
  return (
    <box flexGrow={1} flexDirection="column" justifyContent="center" alignItems="center" backgroundColor={theme.bg} gap={1}>
      <text fg={theme.error}>Terminal too small</text>
      <text fg={theme.fg}>{`${width} columns`}</text>
      <text fg={theme.dim}>need 50</text>
      <text fg={theme.fg}>Resize if you can</text>
      <text fg={theme.dim}>q  esc  ctrl+c</text>
    </box>
  )
}

const bootKind = process.env.KOMA_REMOTE ? "remote" : process.env.KOMA_START === "hub" ? "hub" : "session"
const renderer = await createCliRenderer({
  exitOnCtrlC: false,
  exitSignals: ["SIGTERM", "SIGQUIT", "SIGABRT", "SIGHUP", "SIGPIPE", "SIGBREAK", "SIGBUS"],
  // A drag sends a resize on every pixel. The default debounce restarts on each
  // one, so the last frame stays frozen and the new columns keep ghost text.
  debounceDelay: 0,
  targetFps: 30,
})
// Keep painting through a drag. The signal path still coalesces; this follows
// the real tty size at 24fps and redraws the cells the emulator just exposed.
let followSize = true
setInterval(() => {
  if (!followSize) return
  const width = process.stdout.columns
  const height = process.stdout.rows
  if (!width || !height) return
  if (width === renderer.width && height === renderer.height) return
  try {
    renderer.resize(width, height)
  } catch {
    // The current frame still owns the buffer. The next tick retries.
  }
}, 42)
process.on("SIGINT", () => {
  onInterrupt()
})
renderer.setBackgroundColor(themeFor(launchPalette(bootKind, savedPalette())).bg)
renderer.setTerminalTitle("koma")
const syntaxStyle = SyntaxStyle.fromStyles(
  syntaxStyles(themeFor("dark")),
  renderer.nativeScene!,
)

createRoot(renderer).render(<App syntaxStyle={syntaxStyle} />)
