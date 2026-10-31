import { LocalUsageLane, UsageContext, UsageReader } from "./usage-reader.ts"
import { FormGuard } from "./modes/drafts.ts"
import { Action, Fullscreen, Note } from "./modes/bits.tsx"
import type { SyntaxStyle } from "@opentui/core"
import { useKeyboard, usePaste } from "@opentui/react"
import { useCallback, useEffect, useRef, useState } from "react"
import { Approval, approvalRequest, Chat } from "./chat.tsx"
import { copyText } from "./copy.ts"
import { useColumns } from "./columns.ts"
import { forwardKey } from "./keys.ts"
import { OpeningSplash, WarmingSplash, warmSplash } from "./loading.tsx"
import { AgentViewer, agentAt, SubagentPanel } from "./modes/subagents.tsx"
import { placeOf, selectedOf, wideEnough } from "./modes/place.ts"
import { nativeMenuInput } from "./modes/bits.tsx"
import type { MenuAction } from "./modes/actions.ts"
import { ModeSurface } from "./modes/surface.tsx"
import { foregroundSession, modeName, type RemoteHandoff, type Snapshot } from "./protocol.ts"
import { QuitScreen, quitState } from "./quit.tsx"
import { rewindOf } from "./screens.tsx"
import { DaemonLink } from "./socket.ts"
import { SuspendedScreen } from "./suspended.tsx"
import { themeFor, type Theme } from "./theme.ts"
import { Toast, type LocalNotice } from "./toast.tsx"

export function SessionScreen({
  id,
  initialPalette = "dark",
  syntaxStyle,
  onHome,
  onOpen,
  onExit,
  onRestart,
  onSuspendedExit,
  onRemote,
  onUpdate,
  onPalette,
  ctrlC,
  transport,
}: {
  id: string
  initialPalette?: string
  transport?: import("./transport.ts").DaemonTransport
  syntaxStyle: SyntaxStyle
  onHome: () => void
  onOpen: (id: string) => void
  onExit: () => void
  onRestart?: () => void
  onSuspendedExit?: () => void
  onRemote: (remote: RemoteHandoff) => void
  /** Hands the terminal to the launcher to install and reopen `id`. Local sessions only. */
  onUpdate?: (id: string) => void
  onPalette: (name: string) => void
  ctrlC?: { current: () => void }
}) {
  const [attempt, setAttempt] = useState(0)
  const usageReaderRef = useRef<UsageReader | null>(null)
  const [usageReader, setUsageReader] = useState<UsageReader | null>(null)
  useEffect(() => {
    // A remote connection can only use its remote lane; never instantiate a local reader.
    const lane = transport?.remote ? transport.usage : new LocalUsageLane()
    const reader = lane ? new UsageReader(lane) : null
    usageReaderRef.current = reader
    setUsageReader(reader)
    return () => { reader?.dispose(); if (usageReaderRef.current === reader) usageReaderRef.current = null }
  }, [id, transport, attempt])
  const columns = useColumns()
  const [snap, setSnap] = useState<Snapshot | null>(null)
  const [phase, setPhase] = useState<"connecting" | "live" | "error">("connecting")
  const [error, setError] = useState("")
  const [notice, setNotice] = useState<LocalNotice | null>(null)
  const linkRef = useRef<DaemonLink | null>(null)
  const formGuard = useRef(new FormGuard())
  const [leavePrompt, setLeavePrompt] = useState(false)
  const [leaveSelected, setLeaveSelected] = useState(0)
  const [pending, setPending] = useState(0)
  const snapRef = useRef<Snapshot | null>(null)
  const report = useCallback((text: string) => {
    if (!text) {
      setError("")
      return
    }
    // A live session already has the toast row. The connect screen is the only place
    // that still keeps the message, because there is no chat to hang a toast on.
    if (snapRef.current) setNotice({ id: crypto.randomUUID(), text, kind: "error" })
    else setError(text)
  }, [])
  const phaseRef = useRef(phase)
  const decision = useRef<string | null>(null)
  const send = useCallback((request: unknown) => {
    const link = linkRef.current
    const current = snapRef.current
    if (phaseRef.current !== "live") return
    if (current && request && typeof request === "object" && ("ApproveTool" in request || "PlanDecision" in request)) {
      const fg = foregroundSession(current)
      if (!fg?.awaiting_approval) return
      const identity = `${fg.id}:${fg.pending_tool_calls[fg.tool_idx]?.id ?? fg.tool_idx}`
      if (decision.current === identity) return
      decision.current = identity
    }
    if (link && current && (placeOf(current.global.mode) === "full" || modeName(current.global.mode) === "Model") && request && typeof request === "object") {
      const r = request as Record<string, unknown>
      let action: MenuAction | null = null
      if (r.MenuSelect) action = { Select: r.MenuSelect as Extract<MenuAction, { Select: unknown }>["Select"] }
      else if (r.MenuReplace) action = { Replace: r.MenuReplace as { field: string; value: string } }
      else if (r.MenuAction) action = r.MenuAction as MenuAction
      else if (r.SendKey) action = { Key: r.SendKey as import("./keys.ts").WireKey }
      if (action) {
        if (!(typeof action === "object" && "Replace" in action)) nativeMenuInput.flush?.()
        if ("Key" in Object(action) && typeof action === "object" && "Key" in action && action.Key.code === "Esc" && formGuard.current.canLeave(current.global.mode) && formGuard.current.dirty(current.global.mode)) {
          setLeaveSelected(0)
          setLeavePrompt(true)
          return
        }
        const tracked = action === "Save"
        if (tracked) {setPending((n) => n+1);setError("")}
        void link.sendMenu(modeName(current.global.mode), action).then((error) => { if (error) report(error) }).finally(() => {if (tracked) setPending((n) => Math.max(0,n-1))})
        return
      }
    }
    link?.send(request)
  }, [])
  const sendTracked = useCallback((request: unknown) => {
    return linkRef.current?.sendTracked(request) ?? Promise.resolve("not connected")
  }, [])

  useEffect(() => {
    let dead = false
    formGuard.current = new FormGuard()
    const link = new DaemonLink(
      (next) => {
        if (dead) return
        if (next) formGuard.current.observe(next.global.mode)
        const fg = next ? foregroundSession(next) : null
        const identity = fg?.awaiting_approval ? `${fg.id}:${fg.pending_tool_calls[fg.tool_idx]?.id ?? fg.tool_idx}` : null
        if (decision.current !== identity) decision.current = null
        snapRef.current = next
        setSnap(next)
        setPhase(next ? "live" : "connecting")
      },
      (control) => {
        if (dead) return
        if (control.kind === "exit") {
          onExit()
          return
        }
        if (control.kind === "disconnect") {
          phaseRef.current = "error"
          setError(control.text)
          setPhase("error")
          return
        }
        if (control.kind === "home") {
          onHome()
          return
        }
        if (control.kind === "open") {
          onOpen(control.id)
          return
        }
        if (control.kind === "new") {
          if (control.kill) link.send("QuitDaemon")
          onOpen(crypto.randomUUID())
          return
        }
        if (control.kind === "select") {
          const text = transcriptOf(snapRef.current)
          if (text) {
            copyText(text)
            setNotice({ id: crypto.randomUUID(), text: "copied the transcript" })
          }
          return
        }
        if (control.kind === "remote") {
          onRemote(control.remote)
          return
        }
        if (control.kind === "update") {
          if (transport?.remote || !onUpdate) {
            report("/update only works in a local session. On the host, run `koma update`.")
            return
          }
          link.detach()
          onUpdate(id)
          return
        }
        if (control.kind === "error" && control.text) {
          report(control.text)
          if (!snapRef.current) setPhase("error")
        }
      },
      transport,
    )
    link.onDisconnect = () => usageReaderRef.current?.dispose()
    // Domain replies are observable even when no structural snapshot changes.
    link.events.on("ExtensionOpResult", (result) => { if (!dead && !result.ok && result.error) report(result.error) })
    link.events.on("AgentOp", (result) => { if (!dead && !result.ok && result.error) report(result.error) })
    link.events.on("WebSearchValues", (result) => { if (!dead && result.error) report(result.error) })
    link.events.on("Notifications", ({ reply }) => { if (!dead && reply.error) report(reply.error) })
    linkRef.current = link
    void link.start(id).catch((err: unknown) => {
      if (dead) return
      setError(err instanceof Error ? err.message : String(err))
      setPhase("error")
    })
    return () => {
      dead = true
      link.detach()
      if (linkRef.current === link) linkRef.current = null
    }
  }, [id, transport, attempt])

  phaseRef.current = phase

  useEffect(() => {
    const name = snap?.global.palette
    if (name) onPalette(name)
  }, [snap?.global.palette, onPalette])

  const suspendedExit = onSuspendedExit ?? onExit
  const restart = () => {
    linkRef.current?.detach()
    phaseRef.current = "connecting"
    setPhase("connecting")
    snapRef.current = null
    setSnap(null)
    setError("")
    setNotice(null)
    setLeavePrompt(false)
    decision.current = null
    if (onRestart) onRestart()
    else setAttempt((value) => value + 1)
  }

  const interrupt = useCallback(() => {
    if (phaseRef.current !== "live") { suspendedExit(); return }
    if (quitState(snapRef.current?.global.mode)) return
    send({ SendKey: { code: { Char: "c" }, mods: 2 } })
  }, [send, suspendedExit])

  useEffect(() => {
    if (!ctrlC) return
    ctrlC.current = interrupt
    return () => { ctrlC.current = () => {} }
  }, [ctrlC, interrupt])

  const theme = themeFor(snap?.global.palette ?? initialPalette)
  const session = snap ? foregroundSession(snap) : null
  const mode = snap ? modeName(snap.global.mode) : ""
  const quit = snap ? quitState(snap.global.mode) : null
  const rewind = snap ? rewindOf(snap.global.mode) : null
  const warming = snap ? warmSplash(snap.global.mode) : null
  const place = snap ? placeOf(snap.global.mode) : null
  const subagents = !quit && Boolean(snap?.global.subagents_open) && place == null && rewind == null
  const viewer = !quit && snap?.global.agent_viewer != null && place == null && rewind == null
  const menuKeys = place != null || subagents || viewer
  const chat = quit != null || mode === "Chat" || rewind != null || place === "overlay" || subagents
  const approval = Boolean(session?.awaiting_approval)
  const busy = Boolean(session && (session.working || session.waiting || session.streaming))
  const [exiting, setExiting] = useState<number | null>(null)

  function chooseQuit(index: number) {
    if (exiting != null) return
    if (index === 2) {
      send({ SendKey: { code: "Esc", mods: 0 } })
      return
    }
    setExiting(index)
    if (index === 0) {
      send("QuitDaemon")
      send("Detach")
      onExit()
      return
    }
    send({ SendKey: { code: "Esc", mods: 0 } })
    send("Detach")
    onExit()
  }

  function moveTo(index: number) {
    if (!snap) return
    const from = selectedOf(snap.global.mode)
    const delta = index - from
    if (delta === 0) return
    const code = delta > 0 ? "Down" : "Up"
    const steps = Math.min(Math.abs(delta), 40)
    for (let i = 0; i < steps; i++) send({ SendKey: { code, mods: 0 } })
  }

  function decideLeave(action: MenuAction) {
    setLeavePrompt(false)
    if (snapRef.current) void linkRef.current?.sendMenu(modeName(snapRef.current.global.mode), action).then((error) => { if (error) report(error) })
  }

  usePaste((event) => {
    if (quit || exiting != null || approval && !warming) { event.preventDefault(); event.stopPropagation() }
  })

  useKeyboard((key) => {
    if (key.eventType === "release" || phase === "error") return
    if (exiting != null) { key.preventDefault(); key.stopPropagation(); return }
    if ((key.ctrl && key.name === "c") || key.sequence === "\u0003" || key.name === "ctrl+c") {
      interrupt()
      return
    }
    if (phase !== "live") {
      if (key.name === "escape") onHome()
      return
    }
    if (approval && !quit && !warming && phase === "live") {
      key.preventDefault()
      key.stopPropagation()
      const name = key.name.toLowerCase()
      const call = session?.pending_tool_calls[session.tool_idx]
      const plan = call?.function.name === "plan_ready" || call?.function.name === "mission_ready"
      if (session && (name === "y" || name === "n" || name === "escape" || plan && name === "a")) {
        send(approvalRequest(session, name === "escape" ? "n" : name))
      }
      return
    }
    if (leavePrompt) {
      if (typeof key.preventDefault === "function") key.preventDefault()
      if (key.name === "escape") setLeavePrompt(false)
      else if (key.name === "tab") setLeaveSelected((n)=>(n+(key.shift?2:1))%3)
      else if (key.name === "left") setLeaveSelected((n)=>(n+2)%3)
      else if (key.name === "right") setLeaveSelected((n)=>(n+1)%3)
      else if (key.name === "s") decideLeave("Save")
      else if (key.name === "d") decideLeave({Key:{code:"Esc",mods:0}})
      else if (["enter","return"].includes(key.name)) {
        if (leaveSelected===2) setLeavePrompt(false)
        else decideLeave(leaveSelected===0 ? "Save" : {Key:{code:"Esc",mods:0}})
      }
      return
    }
    if (warming && phase === "live" && !quit && key.name === "escape") {
      if (typeof key.preventDefault === "function") key.preventDefault()
      send({ SendKey: { code: "Esc", mods: 0 } })
      return
    }
    if (rewind && phase === "live" && !quit) {
      const prevent = () => {
        if (typeof key.preventDefault === "function") key.preventDefault()
      }
      if (key.name === "up") {
        prevent()
        send({ SendKey: { code: "Up", mods: 0 } })
      } else if (key.name === "down") {
        prevent()
        send({ SendKey: { code: "Down", mods: 0 } })
      } else if (key.name === "pageup") {
        prevent()
        for (let i = 0; i < 10; i++) send({ SendKey: { code: "Up", mods: 0 } })
      } else if (key.name === "pagedown") {
        prevent()
        for (let i = 0; i < 10; i++) send({ SendKey: { code: "Down", mods: 0 } })
      } else if (key.name === "escape") {
        prevent()
        send({ SendKey: { code: "Esc", mods: 0 } })
      } else if (key.name === "return" || key.name === "enter") {
        prevent()
        send({ SendKey: { code: "Enter", mods: 0 } })
      }
      return
    }
    if (menuKeys && phase === "live" && !quit) {
      if (modeName(snapRef.current?.global.mode) === "Usage" && ["left", "right", "up", "down", "home", "end", "pageup", "pagedown", "r"].includes(key.name)) return
      if (nativeMenuInput.current) {
        const navigation = ["escape", "tab", "return", "enter"].includes(key.name) || nativeMenuInput.multiline && key.ctrl && key.name === "x" || nativeMenuInput.listNavigation && (["up", "down"].includes(key.name) || key.ctrl && ["r", "x", "f"].includes(key.name))
        if (!navigation || nativeMenuInput.multiline && ["return", "enter", "tab"].includes(key.name)) return
      }
      if (typeof key.preventDefault === "function") key.preventDefault()
      const wire = forwardKey(key)
      if (wire) send({ SendKey: wire })
      return
    }
    if (quit && phase === "live") {
      if (exiting != null) return
      if (key.name === "left" || key.name === "h") send({ SendKey: { code: "Left", mods: 0 } })
      else if (key.name === "right" || key.name === "l") send({ SendKey: { code: "Right", mods: 0 } })
      else if (key.name === "tab" && key.shift) send({ SendKey: { code: "BackTab", mods: 0 } })
      else if (key.name === "tab") send({ SendKey: { code: "Tab", mods: 0 } })
      else if (key.name === "k") chooseQuit(0)
      else if (key.name === "d") chooseQuit(1)
      else if (key.name === "escape") chooseQuit(2)
      else if (key.name === "return" || key.name === "enter") chooseQuit(quit.selected)
      return
    }
    if (phase !== "live" || !chat) {
      if (key.name === "escape") onHome()
      return
    }
  })

  if (phase === "connecting") {
    return <OpeningSplash theme={theme} />
  }
  if (phase === "error") {
    return <SuspendedScreen error={error} theme={theme} onRestart={restart} onExit={suspendedExit} />
  }
  if (!snap) return null
  if (warming) {
    return <WarmingSplash theme={theme} warm={warming} />
  }
  if (leavePrompt && !approval && !quit) {
    return <Fullscreen title="Unsaved changes" theme={theme} hint={<box flexDirection="row"><Action label="Save (s)" primary={leaveSelected === 0} theme={theme} onPress={() => decideLeave("Save")} /><Action label="Discard (d)" primary={leaveSelected === 1} theme={theme} onPress={() => decideLeave({ Key: { code: "Esc", mods: 0 } })} /><Action label="Keep editing (esc)" primary={leaveSelected === 2} theme={theme} onPress={() => setLeavePrompt(false)} /></box>}><Note text="Save these changes before leaving?" color={theme.warn} theme={theme} /></Fullscreen>
  }
  if (place === "full") {
    return <box flexDirection="column" flexGrow={1} minHeight={0}>{notice ? <Toast id={notice.id} kind={notice.kind ?? "error"} text={notice.text} theme={theme} onDismiss={() => setNotice((current) => current?.id === notice.id ? null : current)} /> : null}{pending ? <text fg={theme.info}>Waiting for daemon…</text> : null}<UsageContext.Provider value={usageReader && linkRef.current?.usageReader ? { reader: usageReader, session: id } : null}><ModeSurface mode={snap.global.mode} theme={theme} columns={columns} onMove={moveTo} disabled={approval} send={approval ? () => {} : send} /></UsageContext.Provider>{approval && session ? <Approval session={session} theme={theme} send={send} /> : null}</box>
  }
  if (viewer) {
    return <box flexDirection="column" flexGrow={1} minHeight={0}><AgentViewer agent={agentAt(session?.subagents, snap.global.agent_viewer)} theme={theme} />{approval && session ? <Approval session={session} theme={theme} send={send} /> : null}</box>
  }
  if (!chat) {
    return <box flexDirection="column" flexGrow={1} minHeight={0}>{notice ? <Toast id={notice.id} kind={notice.kind ?? "error"} text={notice.text} theme={theme} onDismiss={() => setNotice((current) => current?.id === notice.id ? null : current)} /> : null}<OtherMode name={mode} theme={theme} note="" />{approval && session ? <Approval session={session} theme={theme} send={send} /> : null}</box>
  }
  const overlay = place === "overlay"
    ? <ModeSurface mode={snap.global.mode} theme={theme} columns={columns} onMove={moveTo} send={send} />
    : subagents
      ? (
        <SubagentPanel
          agents={session?.subagents ?? []}
          pending={session?.pending_subagents ?? []}
          selected={snap.global.subagent_sel}
          theme={theme}
          wide={wideEnough(columns)}
          onMove={(index) => {
            const from = snap.global.subagent_sel
            const delta = index - from
            if (delta === 0) return
            const code = delta > 0 ? "Down" : "Up"
            for (let i = 0; i < Math.min(Math.abs(delta), 40); i++) send({ SendKey: { code, mods: 0 } })
          }}
        />
      )
      : null
  return (
    <box flexDirection="column" flexGrow={1} backgroundColor={theme.bg}>
      <Chat
        snap={snap}
        theme={theme}
        syntaxStyle={syntaxStyle}
        focused={!quit && exiting == null && !approval && !rewind && !menuKeys}
        rewind={rewind}
        overlay={overlay}
        modalOverlay={quit ? <QuitScreen state={quit} theme={theme} exiting={exiting} onPick={chooseQuit} /> : null}
        notice={notice}
        onNoticeDismiss={() => setNotice((current) => current?.id === notice?.id ? null : current)}
        send={send}
        sendTracked={sendTracked}
        onFlushError={report}
      />
    </box>
  )
}

function transcriptOf(snap: Snapshot | null): string {
  const session = snap ? foregroundSession(snap) : null
  if (!session) return ""
  return session.messages
    .filter((row) => row.role === "user" || row.role === "assistant")
    .map((row) => row.content)
    .filter((text) => text.trim().length > 0)
    .join("\n\n")
}

function OtherMode({ name, theme, note }: { name: string; theme: Theme; note: string }) {
  return (
    <box flexDirection="column" flexGrow={1} padding={1} gap={1} backgroundColor={theme.bg}>
      <text fg={theme.accent}>{name}</text>
      <text>This screen is still on koma legacy.</text>
      {note ? <text fg={theme.warn}>{note}</text> : null}
      <text fg={theme.dim}>esc home</text>
    </box>
  )
}
