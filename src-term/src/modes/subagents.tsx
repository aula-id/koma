import { useRef } from "react"
import { useKeyboard } from "@opentui/react"
import type { ScrollBoxRenderable } from "@opentui/core"
import { BORDER_BOTTOM, scrollbars, type Theme } from "../theme.ts"
import { Lines, Note, OVERLAY_BODY, OVERLAY_HEIGHT, Paragraphs, type Line } from "./bits.tsx"
import { clip, flag, num, rec, rows, str, type Rec } from "./read.ts"

export function SubagentPanel({
  agents,
  pending,
  selected,
  theme,
  wide,
  onMove,
}: {
  agents: Rec[]
  pending: Rec[]
  selected: number
  theme: Theme
  wide: boolean
  onMove: (index: number) => void
}) {
  const lines: Line[] = [
    ...agents.map((row) => ({ label: str(row.label) || str(row.name), detail: str(row.status) })),
    ...pending.map((row) => ({ label: str(row.agent_name), detail: "queued", dim: true })),
  ]
  const current = agents[selected]
  const live = str(current?.live_text)
  const detail = current
    ? [str(current.status), flag(current.detached) ? "background" : "attached", clip(live, 200) || " "]
    : ["no sub-agents"]
  return (
    <box border borderStyle="rounded" borderColor={theme.dim} flexDirection="column" height={OVERLAY_HEIGHT} flexShrink={0} title=" sub-agents " titleColor={theme.dim}>
      <box flexDirection="row" flexGrow={1} minHeight={0}>
        <box width={wide ? 28 : 18} flexGrow={1} minHeight={0} flexDirection="column">
          <Lines rows={lines} selected={selected} theme={theme} visible={OVERLAY_BODY} rail onPress={onMove} />
        </box>
        <box flexGrow={1} paddingLeft={1}>
          <Paragraphs lines={detail.slice(0, OVERLAY_BODY)} theme={theme} />
        </box>
      </box>
      <text fg={theme.dim}> ↑↓ · ctrl+x kill · esc close</text>
    </box>
  )
}

export function AgentViewer({ agent, theme }: { agent: Rec | null; theme: Theme }) {
  const messages = rows(agent?.messages)
  const lines = messages.map((row) => `${str(row.role)}\n${str(row.content)}`)
  const scroll = useRef<ScrollBoxRenderable | null>(null)
  useKeyboard((key) => {
    if (key.eventType === "release") return
    if (key.name === "up") scroll.current?.scrollBy(-1)
    if (key.name === "down") scroll.current?.scrollBy(1)
    if (key.name === "pageup") scroll.current?.scrollBy(-1, "viewport")
    if (key.name === "pagedown") scroll.current?.scrollBy(1, "viewport")
  })
  const live = str(agent?.live_text)
  return (
    <box flexGrow={1} flexDirection="column" backgroundColor={theme.bg}>
      <text fg={theme.dim}>{` ${str(agent?.label) || str(agent?.name) || "sub-agent"}`}</text>
      <box height={1} border={BORDER_BOTTOM} borderColor={theme.dim} />
      <box flexGrow={1} flexDirection="column" paddingLeft={1} paddingTop={1}>
<scrollbox ref={scroll} flexGrow={1} minHeight={0} scrollY focusable={false} scrollbarOptions={scrollbars(theme)}>{lines.map((line, i) => <text key={i} fg={theme.fg}>{line}</text>)}{live ? <text fg={theme.fg}>{live}</text> : null}</scrollbox>
      </box>
      <text fg={theme.dim}> ↑↓ scroll · esc back</text>
    </box>
  )
}

export function agentAt(agents: unknown, index: number | null): Rec | null {
  if (index == null) return null
  const list = rows(agents)
  return list[index] ? rec(list[index]) : null
}

export function asAgents(value: unknown): Rec[] {
  return rows(value)
}

export { num }
