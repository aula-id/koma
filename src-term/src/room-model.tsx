import { useState } from "react"
import { useKeyboard, useTerminalDimensions } from "@opentui/react"
import type { Theme } from "./theme.ts"
import { Choice } from "./chat.tsx"
import { Panel } from "./modes/bits.tsx"
import { paletteWindow } from "./palette.ts"
import { filterModels, minions } from "./room-model.ts"
import type { LaneRow, ModelChoice } from "./room-link.ts"

type Step = "minion" | "lane" | "model"
type Row = { key: string; label: string; detail: string }

/** Pick a minion, then one of its lanes, then a model registered in the coding agent. */
export function ModelPicker({ theme, choices, lanes, notice, onSet, onBack }: {
  theme: Theme
  choices: ModelChoice[]
  lanes: LaneRow[]
  notice: string
  onSet: (avatar: string, lane: string, model: string) => void
  onBack: () => void
}) {
  const { height } = useTerminalDimensions()
  const team = minions(lanes)
  const [step, setStep] = useState<Step>("minion")
  const [at, setAt] = useState(0)
  const [who, setWho] = useState("")
  const [lane, setLane] = useState("")
  const [query, setQuery] = useState("")
  const [done, setDone] = useState("")

  const minion = team.find((m) => m.avatar === who)
  const current = minion?.lanes.find((l) => l.lane === lane)?.model ?? ""
  const matches = filterModels(choices, query)

  const rows: Row[] =
    step === "minion"
      ? team.map((m) => ({ key: m.avatar, label: m.display || m.avatar, detail: m.lanes.map((l) => `${l.lane}: ${l.model}`).join("  ·  ") }))
      : step === "lane"
        ? (minion?.lanes ?? []).map((l) => ({ key: l.lane, label: l.lane, detail: l.model }))
        : [
            ...(query ? [] : [{ key: "", label: "session default", detail: "inherit the model of this session" }]),
            ...matches.map((c) => ({ key: c.id, label: c.name, detail: [c.provider, c.model_id].filter(Boolean).join(" · ") })),
          ]
  const selected = Math.min(at, Math.max(0, rows.length - 1))
  const visible = Math.max(3, height - (step === "model" ? 11 : 8))
  const start = paletteWindow(0, selected, rows.length, visible)
  const shown = rows.slice(start, start + visible)

  const back = () => {
    if (step === "model") { setStep(minion && minion.lanes.length > 1 ? "lane" : "minion"); setQuery(""); setAt(0) }
    else if (step === "lane") { setStep("minion"); setAt(0) }
    else onBack()
  }
  const choose = (index: number) => {
    const row = rows[index]
    if (!row) return
    if (step === "minion") {
      const m = team.find((t) => t.avatar === row.key)
      if (!m) return
      setWho(m.avatar)
      if (m.lanes.length === 1) { setLane(m.lanes[0]!.lane); setStep("model") }
      else setStep("lane")
      setAt(0)
    } else if (step === "lane") {
      setLane(row.key)
      setStep("model")
      setQuery("")
      setAt(0)
    } else {
      onSet(who, lane, row.key)
      setDone(`${minion?.display || who} · ${lane} → ${row.label}`)
      setStep("minion")
      setQuery("")
      setAt(0)
    }
  }

  useKeyboard((key) => {
    if (key.eventType === "release") return
    if (key.name === "escape") return back()
    if (key.name === "up") return setAt((selected - 1 + Math.max(1, rows.length)) % Math.max(1, rows.length))
    if (key.name === "down") return setAt((selected + 1) % Math.max(1, rows.length))
    if (key.name === "pageup") return setAt(Math.max(0, selected - visible + 1))
    if (key.name === "pagedown") return setAt(Math.min(Math.max(0, rows.length - 1), selected + visible - 1))
    // In the model step the search input owns Enter.
    if (step !== "model" && (key.name === "return" || key.name === "enter")) choose(selected)
  })

  const title = step === "minion" ? "Which minion" : step === "lane" ? `${minion?.display || who} · which lane` : `${minion?.display || who} · ${lane} model`
  const place = rows.length > visible ? ` · ${start + 1}-${start + shown.length} of ${rows.length}` : ""
  const status = step === "model" ? `now: ${current}${place}` : (done || notice || place.slice(3)) || undefined

  return (
    <box
      flexDirection="column" flexGrow={1} backgroundColor={theme.bg} paddingLeft={2} paddingRight={2} paddingTop={1}
      onMouseScroll={(event: { scroll?: { direction?: string } }) => {
        const d = event.scroll?.direction
        if (d === "up") setAt(Math.max(0, selected - 3))
        else if (d === "down") setAt(Math.min(Math.max(0, rows.length - 1), selected + 3))
      }}
    >
      <box flexDirection="row" justifyContent="space-between" flexShrink={0}>
        <text flexShrink={0}>
          <span fg={theme.fg}>Koma</span>
          <span fg={theme.dim}> | </span>
          <span fg={theme.accent}>secure room · model</span>
        </text>
        <text fg={theme.dim} onMouseDown={back}>Back (esc)</text>
      </box>
      <box flexDirection="column" flexGrow={1} flexBasis={0} minHeight={0} paddingTop={1}>
        {step === "model" ? (
          <box height={3} flexShrink={0} border borderStyle="rounded" borderColor={theme.accent} title=" Search " titleColor={theme.accent}>
            <input
              value={query}
              focused
              placeholder="Name, model id or provider"
              placeholderColor={theme.dim}
              textColor={theme.fg}
              focusedTextColor={theme.fg}
              backgroundColor={theme.bg}
              focusedBackgroundColor={theme.bg}
              cursorColor={theme.accent}
              onInput={(value: string) => { setQuery(value); setAt(0) }}
              onSubmit={() => choose(selected)}
            />
          </box>
        ) : null}
        <Panel title={title} hot={step !== "model"} theme={theme} status={status}>
          {rows.length === 0 ? (
            <text fg={theme.dim} paddingLeft={1}>{step === "model" ? (choices.length === 0 ? "No model is registered in the coding agent. Add one with /settings." : "Nothing matches.") : "The room has not listed its minions yet."}</text>
          ) : shown.map((row, index) => (
            <Choice key={`${row.key}-${start + index}`} label={row.label} detail={row.detail} theme={theme} selected={start + index === selected} onPress={() => choose(start + index)} />
          ))}
        </Panel>
      </box>
    </box>
  )
}
