import { useEffect, useRef, useState, type ReactNode } from "react"
import type { TextareaAction, TextareaRenderable } from "@opentui/core"
import { useRenderer, useTerminalDimensions } from "@opentui/react"
import { COMPOSER_KEYS, PALETTE_KEYS, Tips, promptMark } from "./chat.tsx"
import type { Theme } from "./theme.ts"

export const ROOM_TIPS = [
  "Message the Chief. It does quick work itself and hands long jobs to the specialists",
  "@red talks to a specialist directly",
  "!pwd runs a shell command right away, without the model",
  "Shift+Enter or Ctrl+J starts a new line",
  "Tab completes a command or a name",
  "/status shows every lane, /shelf the findings, /trajectory the goals",
  "/model picks the model of each minion's lane",
  "Ctrl+C asks before leaving the room",
]

type Keys = { name: string; action: TextareaAction; ctrl?: boolean; shift?: boolean; meta?: boolean }[]

/**
 * The secure room's composer. It matches the coding chat's: a box that grows with the
 * draft, the Koma row above it with a status on the right, the tips line, and the
 * suggestion popup in the slot flush above the box. The draft lives in the screen.
 */
export function RoomComposer({
  theme,
  draft,
  onDraft,
  onSubmit,
  focused,
  ready,
  status,
  popup,
  popupOpen,
  placeholder,
}: {
  theme: Theme
  draft: string
  onDraft: (text: string) => void
  onSubmit: () => void
  focused: boolean
  ready: boolean
  status: { text: string; color: string }
  popup?: ReactNode
  popupOpen: boolean
  placeholder: string
}) {
  const { height, width } = useTerminalDimensions()
  const renderer = useRenderer()
  const area = useRef<TextareaRenderable | null>(null)
  const [lines, setLines] = useState(Math.max(1, draft.split("\n").length))
  const [wrapped, setWrapped] = useState(1)
  const innerCap = Math.max(1, Math.floor(height / 2) - 2)
  const boxHeight = Math.min(innerCap, Math.max(1, lines, wrapped)) + 2

  // The screen owns the draft. When it changes the text (a completion, a send), show it.
  // A draft that is just an older echo of what was typed is not a change: fast typing
  // would otherwise be overwritten by the screen's stale copy.
  // The textarea keeps the handlers it was first given, so they reach the latest props by ref.
  const submitRef = useRef(onSubmit)
  submitRef.current = onSubmit
  const draftRef = useRef(onDraft)
  draftRef.current = onDraft
  const emitted = useRef<string[]>([])
  useEffect(() => {
    const current = area.current
    const echo = emitted.current.indexOf(draft)
    if (echo >= 0) {
      emitted.current = emitted.current.slice(echo + 1)
      return
    }
    emitted.current = []
    if (!current || current.plainText === draft) return
    current.setText(draft)
    current.cursorOffset = draft.length
    setLines(Math.max(1, draft.split("\n").length))
  }, [draft])

  useEffect(() => {
    const read = () => {
      const count = Math.max(1, area.current?.editorView.getTotalVirtualLineCount() ?? 1)
      setLines((previous) => previous === count ? previous : count)
    }
    renderer.on("frame", read)
    return () => { renderer.off("frame", read) }
  }, [renderer])
  useEffect(() => {
    const count = Math.max(1, area.current?.virtualLineCount ?? 1)
    setWrapped((current) => current === count ? current : count)
  }, [draft, width])

  const keys = (popupOpen ? [...COMPOSER_KEYS, ...PALETTE_KEYS] : COMPOSER_KEYS) as Keys
  return (
    <box flexDirection="column" flexShrink={0}>
      <box flexDirection="row" justifyContent="space-between" paddingLeft={1} paddingRight={1}>
        <text flexShrink={0}>
          <span fg={theme.fg}>Koma</span>
          <span fg={theme.dim}> | </span>
          <span fg={theme.accent}>secure room</span>
        </text>
        <text fg={status.color} wrapMode="none">{status.text}</text>
      </box>
      <Tips theme={theme} width={width} tips={ROOM_TIPS} />
      <box flexDirection="column" flexShrink={0}>{popup}</box>
      <box height={boxHeight} flexShrink={0}>
        <box
          border
          borderStyle="rounded"
          borderColor={focused && ready ? theme.accent : theme.dim}
          title=" secure "
          titleAlignment="right"
          titleColor={theme.accent}
          flexGrow={1}
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
            focused={focused}
            wrapMode="word"
            {...{ focusable: true }}
            initialValue={draft}
            placeholder={placeholder}
            placeholderColor={theme.dim}
            textColor={theme.fg}
            focusedTextColor={theme.fg}
            backgroundColor={theme.bg}
            focusedBackgroundColor={theme.bg}
            cursorColor={theme.accent}
            keyBindings={keys}
            onContentChange={() => {
              const text = area.current?.plainText ?? ""
              emitted.current = [...emitted.current.slice(-50), text]
              draftRef.current(text)
            }}
            onSubmit={() => submitRef.current()}
          />
        </box>
      </box>
    </box>
  )
}
