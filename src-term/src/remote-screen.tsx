import type { SyntaxStyle } from "@opentui/core"
import { useKeyboard } from "@opentui/react"
import { useEffect, useRef, useState } from "react"
import { Action, Fullscreen, InputField, Lines, Note, Panel, nativeMenuInput, type Line } from "./modes/bits.tsx"
import { RemoteBridge, type RemoteEvent, type RemoteSession } from "./remote.ts"
import type { RemoteHandoff } from "./protocol.ts"
import { SessionScreen } from "./session.tsx"
import { SuspendedScreen } from "./suspended.tsx"
import type { Theme } from "./theme.ts"
import { parentOf } from "./remote-path.ts"

type Stage = "progress" | "auth" | "sessions" | "directories" | "live" | "error"
export function RemoteScreen({ remote, theme, palette = "dark", syntaxStyle, onHome, onExit, onRemote, onPalette, ctrlC }: {
  remote: RemoteHandoff; theme: Theme; palette?: string; syntaxStyle: SyntaxStyle; onHome: () => void; onExit: () => void; onRemote: (remote: RemoteHandoff) => void; onPalette: (palette: string) => void; ctrlC: { current: () => void }
}) {
  const resumeId = useRef(remote.sessionId)
  const bridge = useRef<RemoteBridge | null>(null)
  const [attempt, setAttempt] = useState(0)
  const [stage, setStage] = useState<Stage>("progress")
  const [progress, setProgress] = useState("Authenticating")
  const [error, setError] = useState("")
  const [password, setPassword] = useState("")
  const [sessions, setSessions] = useState<{ live: RemoteSession[]; history: RemoteSession[] }>({ live: [], history: [] })
  const [group, setGroup] = useState<"live" | "history">("live")
  const [selected, setSelected] = useState<string | null>(null)
  const [path, setPath] = useState("/")
  const [loadedPath, setLoadedPath] = useState("/")
  const [dirs, setDirs] = useState<string[]>([])
  const [directoryPending,setDirectoryPending]=useState(false)
  const [dirSel, setDirSel] = useState(0)
  const [directoryFocus, setDirectoryFocus] = useState(false)
  const [sessionId, setSessionId] = useState("")
  const [armed, setArmed] = useState<{ id: string; name: string; group: "live" | "history" } | null>(null)
  useEffect(() => {
    let dead = false
    const link = new RemoteBridge((event: RemoteEvent) => {
      if (dead) return
      if (event === "Disconnected") return
      if ("Progress" in event) { setProgress(event.Progress.stage); setStage("progress") }
      else if ("AuthRequired" in event) { setPassword(""); setError(event.AuthRequired.error || ""); setStage("auth") }
      else if ("Sessions" in event) { setSessions(event.Sessions); setStage("sessions"); setError("") }
      else if ("Directories" in event) { setPath(event.Directories.path); setLoadedPath(event.Directories.path); setDirectoryPending(false);setDirs(event.Directories.entries); setDirSel(0); setStage("directories"); setError("") }
      else if ("Connected" in event) { resumeId.current = event.Connected.session_id; setSessionId(event.Connected.session_id); setStage("live"); setError("") }
      else if ("ConnectionLost" in event) { setError(event.ConnectionLost.message); setStage("error"); link.dispose() }
      else if ("Error" in event) { setDirectoryPending(false);setError(event.Error.message); setStage((old) => old === "sessions" || old === "directories" ? old : "error") }
    })
    bridge.current = link
    setStage("progress")
    link.start({ ...remote, sessionId: resumeId.current })
    return () => { dead = true; link.dispose(); if (bridge.current === link) bridge.current = null }
  }, [remote, attempt])
  useEffect(() => {
    if (stage !== "sessions") return
    const timer = setInterval(() => bridge.current?.command("Sessions"), 2000)
    return () => clearInterval(timer)
  }, [stage])
  useEffect(() => {
    if (stage === "live") return
    ctrlC.current = onExit
    return () => { ctrlC.current = () => {} }
  }, [stage, ctrlC, onExit])
  const restart = () => { bridge.current?.dispose(); setStage("progress"); setError(""); setAttempt((value) => value + 1) }
  const list = sessions[group]
  const index = Math.max(0, list.findIndex((s) => s.session_id === selected))
  const current = list[index]
  const goHub = () => { bridge.current?.command("Disconnect"); bridge.current?.command("Sessions"); setStage("progress"); setProgress("Loading sessions") }
  const open = (id: string | null, cwd: string | null = null) => { bridge.current?.command({ Open: { session_id: id, cwd } }); setStage("progress"); setProgress("Connecting") }
  const authenticate = () => { bridge.current?.command({ Password: { value: password } }); setError(""); setPassword(""); setStage("progress"); setProgress("Authenticating") }
  const loadDirs = (value: string) => { bridge.current?.command({ Directories: { path: value } }); setDirectoryPending(true);setStage("directories"); setProgress("Listing remote directories") }
  const remove = () => {
    if (!current) return
    if (!armed) { setArmed({ id: current.session_id, name: current.name, group }); return }
    bridge.current?.command(armed.group === "live" ? { Kill: { session_id: armed.id } } : { Delete: { session_id: armed.id } })
    setArmed(null)
  }
  useKeyboard((key) => {
    if (key.eventType === "release" || stage === "live" || stage === "error") return
    if (key.ctrl && key.name === "c" || key.sequence === "\u0003" || key.name === "ctrl+c") { onExit(); return }
    if (key.name === "escape") { if (armed) setArmed(null); else onHome(); return }
    if (key.name === "tab") { if (stage === "sessions") setGroup((g) => g === "live" ? "history" : "live"); else if (stage === "directories") setDirectoryFocus((old) => !old); return }
    if (nativeMenuInput.current && !["return", "enter"].includes(key.name) && !(stage === "directories" && ["up", "down"].includes(key.name))) return
    if (["return", "enter"].includes(key.name)) {
      if (stage === "auth") authenticate()
      else if (armed) remove()
      else if (stage === "sessions" && current) open(current.session_id)
      else if (stage === "directories") { if (!directoryFocus && path !== loadedPath) loadDirs(path); else if (!directoryFocus && dirs[dirSel]) loadDirs(dirs[dirSel]!); else open(null, loadedPath) }
    }
    if (stage === "sessions" && (key.name === "up" || key.name === "down")) { setSelected(list[Math.max(0, Math.min(list.length - 1, index + (key.name === "down" ? 1 : -1)))]?.session_id ?? null); setArmed(null) }
    if (stage === "sessions" && key.ctrl && key.name === "x") remove()
    if (stage === "directories" && ["up", "down"].includes(key.name)) setDirSel((old) => Math.max(0, Math.min(dirs.length - 1, old + (key.name === "down" ? 1 : -1))))
  })
  if (stage === "error") return <SuspendedScreen error={error} theme={theme} onRestart={restart} onExit={onExit} />
  if (stage === "live" && bridge.current) return <SessionScreen key={sessionId} id={sessionId} initialPalette={palette} transport={bridge.current.transport} syntaxStyle={syntaxStyle} onHome={goHub} onOpen={(id) => open(id)} onExit={onHome} onRestart={restart} onSuspendedExit={onExit} onRemote={onRemote} onPalette={onPalette} ctrlC={ctrlC} />
  const actions = <box flexDirection="row" flexWrap="wrap" flexShrink={0}>
    {stage === "auth" ? <Action label="Authenticate (enter)" primary theme={theme} onPress={authenticate} /> : null}
    {stage === "sessions" ? <><Action label={armed ? `Confirm ${armed.group === "live" ? "kill" : "delete"} (enter)` : "Open (enter)"} theme={theme} onPress={() => { if (armed) remove(); else if (current) open(current.session_id) }} /><Action label="New session" theme={theme} onPress={() => loadDirs("/")} /><Action label={group === "live" ? "History (tab)" : "Live (tab)"} theme={theme} onPress={() => setGroup((g) => g === "live" ? "history" : "live")} /><Action label={group === "live" ? "Kill (ctrl+x)" : "Delete (ctrl+x)"} theme={theme} onPress={remove} /></> : null}
    {stage === "directories" ? <><Action label="Inspect path" theme={theme} onPress={() => loadDirs(path)} /><Action label="Parent" theme={theme} onPress={() => loadDirs(parentOf(loadedPath))} /><Action label="Select folder" primary={directoryFocus} theme={theme} onPress={() => open(null, loadedPath)} /></> : null}
    <Action label="Back (esc)" theme={theme} onPress={() => { if (armed) setArmed(null); else onHome() }} />
  </box>
  const rows: Line[] = list.map((s) => ({ label: s.name || s.session_id, detail: `${s.pwd} · ${s.session_id.slice(0, 8)}`, badge: { text: s.working ? "working" : group === "live" ? "ready" : "saved", role: s.working ? "success" : "info" } }))
  return <Fullscreen title={`Remote / ${remote.target}`} hint={actions} theme={theme}>
    {error ? <Note text={error} color={theme.error} theme={theme} /> : null}
    {stage === "progress" ? <Note text={progress} color={theme.info} theme={theme} /> : null}
    {stage === "auth" ? <InputField key={`password-${attempt}`} label="SSH password" value="" active secret onChange={setPassword} theme={theme} /> : null}
    {stage === "sessions" ? <><Note text={armed ? `${armed.group === "live" ? "Stop" : "Delete"} ${armed.name} (${armed.id})?` : `${group} · ${list.length} sessions`} theme={theme} color={armed ? theme.warn : theme.dim} /><Lines rows={rows} selected={index} onPress={(i) => setSelected(list[i]!.session_id)} theme={theme} /></> : null}
    {stage === "directories" ? <>{directoryPending ? <Note text="Listing remote directories…" color={theme.info} theme={theme} /> : null}<InputField key={loadedPath} field="directory" label="Remote working directory" value={path} active={!directoryFocus} onChange={setPath} theme={theme} /><Panel title="Directories" hot={!directoryFocus} theme={theme}><Lines rows={dirs.map((label) => ({ label }))} selected={dirSel} theme={theme} onPress={(i) => { setDirSel(i); loadDirs(dirs[i]!) }} /></Panel></> : null}
  </Fullscreen>
}
