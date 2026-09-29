import {
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
  type ReactNode,
} from 'react'
import { ArrowUp, Layers, Paperclip, Search, Square, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import {
  readCodingPathDragData,
} from '../lib/codingRef'
import { diagramViewForMermaid, mermaidTitle, splitDiagramMessage } from '../lib/diagramMermaid'
import {
  assignFreshPasteMarkers,
  formatPasteFence,
  PASTE_SOFT_MAX_BYTES,
  pasteByteLength,
  pasteMarker,
  shouldCollapsePaste,
  splitPasteMessage,
  type PasteMarkerRow,
  type PastedBlock,
} from '../lib/pasteText'
import type { DiagramDoc } from '../lib/diagram'
import { ModelPicker } from './ModelPicker'
import { EffortPicker } from './EffortPicker'
import { ModeSelector } from './ModeSelector'
import { CatMascot } from './CatMascot'
import { DiagramSketch } from './DiagramVisual'

type DiagramChip = { id: string; title: string; mermaid: string; doc: DiagramDoc }

function mintDiagramChipId(): string {
  return `d${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`
}

type LocalPaste = PastedBlock & { id: string; markerN?: number }

function steerPreview(text: string): string {
  const pasted = splitPasteMessage(text)
  const split = splitDiagramMessage(pasted.prose)
  return [
    split.prose,
    ...split.diagrams.map((item) => mermaidTitle(item.mermaid)),
    ...pasted.pastes.map((item) => `Pasted Text #${item.n}`),
  ]
    .map((part) => part.replace(/\s+/g, ' ').trim())
    .filter(Boolean)
    .join(' · ')
}

function chipsFromMessage(text: string): { prose: string; chips: DiagramChip[]; pastes: LocalPaste[] } {
  const pasted = splitPasteMessage(text)
  const split = splitDiagramMessage(pasted.prose)
  return {
    prose: split.prose,
    chips: split.diagrams.map((item) => {
      const view = diagramViewForMermaid(item.mermaid)
      return { id: mintDiagramChipId(), title: mermaidTitle(item.mermaid), mermaid: item.mermaid, doc: view.doc }
    }),
    pastes: pasted.pastes.map((item) => ({ ...item, id: mintDiagramChipId() })),
  }
}
// Build-time JSON import: src-misc/ lives outside the vite root (src-webgui/)
// but is the Rust side's single source of truth for this word list, so it's
// imported directly rather than copied — vite/rollup inlines it into the
// bundle at build time (fs.allow only gates the dev-server's HTTP file
// serving, not the module graph read via Node fs during build).
import wanderWords from '../../../src-misc/wanderer.json'

// Reads a File's bytes and resolves to a bare base64 string (no `data:` URL
// prefix) for the AttachFile GuiReq.
function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const result = reader.result as string
      const comma = result.indexOf(',')
      resolve(comma >= 0 ? result.slice(comma + 1) : result)
    }
    reader.onerror = () => reject(reader.error)
    reader.readAsDataURL(file)
  })
}

// --- Inline file-ref chips ---------------------------------------------------
// TUI parity: picking a file in OmniSearchPalette inserts a `@<label>` token
// (see OmniSearchPalette.tsx) — the same wire text the TUI composer produces
// and sends verbatim to the model. This composer paints those tokens as
// inline pills (Google-Docs-style) via a transparent-text textarea layered
// over a mirrored overlay.
//
// Chips are tracked as SUBSTRING RANGES, not whitespace-delimited tokens — a
// real filename label can itself contain a space ("My Notes.md" -> the
// inserted text is `@My Notes.md `, which whitespace-splits into TWO runs,
// "@My" and "Notes.md"). Splitting on whitespace would silently break both
// the pill and the atomic delete for any such label, so instead:
//  1. Scan for exact, non-overlapping occurrences of every token this session
//     inserted (pickedTokensRef, longest tokens matched first so a token that
//     happens to be a prefix/substring of another can't shadow-steal it).
//  2. Layer in `@[<n>]<path>` multi-root-sentinel matches (a SEPARATE,
//     non-anchored regex pass over the raw text, not a whitespace-token
//     test) for any range not already claimed by (1) — this is what keeps a
//     multi-root chip recognizable after a reload/history-recall even though
//     pickedTokensRef itself doesn't survive either (accepted asymmetry: a
//     single-root label with no `[N]` prefix has no shape to fall back on,
//     so it loses its chip after a reload).
// The resulting ranges drive both the overlay renderer and the atomic-delete
// keydown handler below — one source of truth for "what's a chip".

// All non-overlapping chip ranges in `text`, sorted by start offset.
function findChipRanges(text: string, pickedTokens: Set<string>): Array<[number, number]> {
  const ranges: Array<[number, number]> = []
  const isClaimed = (s: number, e: number) => ranges.some(([rs, re]) => s < re && e > rs)

  // Pass 1: exact picked-token substrings, longest first.
  const tokens = Array.from(pickedTokens)
    .filter((t) => t.length > 0)
    .sort((a, b) => b.length - a.length)
  for (const token of tokens) {
    let from = 0
    while (from <= text.length - token.length) {
      const idx = text.indexOf(token, from)
      if (idx === -1) break
      const end = idx + token.length
      if (!isClaimed(idx, end)) ranges.push([idx, end])
      from = idx + 1
    }
  }

  // Pass 2: multi-root sentinel shape, anywhere it isn't already claimed.
  const sentinelRe = /@\[\d+\]\S+/g
  let m: RegExpExecArray | null
  while ((m = sentinelRe.exec(text))) {
    const idx = m.index
    const end = idx + m[0].length
    if (!isClaimed(idx, end)) ranges.push([idx, end])
  }

  ranges.sort((a, b) => a[0] - b[0])
  return ranges
}

// Backspace: fires for the range the caret sits AFTER or INSIDE
// (start < pos <= end) — there has to be actual chip text immediately to the
// caret's left, not just a chip that happens to start right at the caret.
function chipRangeForBackspace(
  ranges: Array<[number, number]>,
  pos: number,
): [number, number] | null {
  return ranges.find(([s, e]) => pos > s && pos <= e) ?? null
}

// Delete: fires for the range the caret sits BEFORE or INSIDE
// (start <= pos < end).
function chipRangeForDelete(
  ranges: Array<[number, number]>,
  pos: number,
): [number, number] | null {
  return ranges.find(([s, e]) => pos >= s && pos < e) ?? null
}

// Shared typography for the transparent textarea + its mirrored chip overlay.
// MUST stay identical on both layers — any padding/line-height/wrap mismatch
// drifts the caret vs painted text. Integer line-height (not leading-relaxed's
// 1.625 × 14px = 22.75) avoids cumulative subpixel rounding that shows up as
// caret misalignment after ~8–10 lines.
const COMPOSER_FIELD_CLASS =
  'm-0 box-border w-full whitespace-pre-wrap break-words p-0 text-[14px] leading-[22px] [overflow-wrap:anywhere] [tab-size:4]'

// Overlay renderer: walks the chip ranges in order, emitting the untouched
// in-between text verbatim and wrapping each range's slice in a tinted pill
// span. The plain-text pieces + pill contents concatenate back to EXACTLY
// `text` — this must stay character-identical (same glyphs, same wrapping),
// since it paints directly behind the transparent textarea and has to line
// up with the real caret/selection pixel-for-pixel.
//
// A trailing `\n` does not paint an empty line in a normal block box the way
// a textarea does, so we append a zero-width space after the content. That
// keeps line boxes (and caret row) aligned without changing visible glyphs.
function renderComposerOverlay(text: string, pickedTokens: Set<string>): ReactNode {
  if (text === '') return null
  const ranges = findChipRanges(text, pickedTokens)
  const tail = '\u200b'
  if (ranges.length === 0) return (
    <>
      {text}
      {tail}
    </>
  )
  const nodes: ReactNode[] = []
  let cursor = 0
  ranges.forEach(([start, end], i) => {
    if (start > cursor) nodes.push(text.slice(cursor, start))
    const part = text.slice(start, end)
    // Dim the `@` (and multi-root `[N]`) prefix inside the pill; the rest of
    // the label reads at normal (tinted) text color. Purely cosmetic — `part`
    // itself (unsplit) is what was matched/compared above.
    const m = part.match(/^(@(?:\[\d+\])?)([\s\S]*)$/)
    nodes.push(
      <span key={i} className="rounded-[4px] bg-koma-accent/15 px-[2px] -mx-[2px] text-koma-fg">
        {m ? (
          <>
            <span className="opacity-50">{m[1]}</span>
            {m[2]}
          </>
        ) : (
          part
        )}
      </span>,
    )
    cursor = end
  })
  if (cursor < text.length) nodes.push(text.slice(cursor))
  nodes.push(tail)
  return nodes
}

// Composer: message textarea + send, plus attach affordances (file-picker
// button, drag-drop onto the composer, clipboard-image paste) and the
// attached-file chip row. Split out of ChatView so the message-rendering
// region there (owned by a parallel branch) stays untouched by this work.
export function Composer() {
  const working = useKoma((s) => s.session.working)
  const attachments = useKoma((s) => s.session.attachments)
  const pendingSteer = useKoma((s) => s.session.pendingSteer)
  const refillComposer = useKoma((s) => s.refillComposer)
  const req = useKoma((s) => s.req)
  // Local selection index for the follow-ups list (client-only; daemon keeps
  // its own for the TUI). Clamped whenever the queue shrinks.
  const [steerSel, setSteerSel] = useState(0)
  const [steerFocus, setSteerFocus] = useState(false)
  const openOmniSearch = useKoma((s) => s.openOmniSearch)
  const omnisearchOpen = useKoma((s) => s.ui.omnisearchOpen)
  const composerInsert = useKoma((s) => s.ui.composerInsert)
  const consumeComposerInsert = useKoma((s) => s.consumeComposerInsert)
  const composerRefill = useKoma((s) => s.ui.composerRefill)
  const consumeComposerRefill = useKoma((s) => s.consumeComposerRefill)
  const pendingRewindIndex = useKoma((s) => s.ui.pendingRewindIndex)
  const clearRewind = useKoma((s) => s.clearRewind)
  const requestScrollBottom = useKoma((s) => s.requestScrollBottom)
  const [input, setInput] = useState('')
  const [diagramChips, setDiagramChips] = useState<DiagramChip[]>([])
  const diagramChipsRef = useRef<DiagramChip[]>([])
  diagramChipsRef.current = diagramChips
  const [localPastes, setLocalPastes] = useState<LocalPaste[]>([])
  const localPastesRef = useRef<LocalPaste[]>([])
  localPastesRef.current = localPastes
  // Fresh pastes wait here until the snapshot assigns `[Pasted Text #N]`.
  // Recalled fences are not queued: they already have a body to splice back.
  const attachQueue = useRef<PasteMarkerRow[]>([])
  const seenPasteMarkers = useRef(new Set<number>())
  const submitArmed = useRef(false)
  const submitRef = useRef<() => void>(() => {})
  const [pasteTexts, setPasteTexts] = useState<Record<number, string>>({})
  const pasteTextsRef = useRef(pasteTexts)
  pasteTextsRef.current = pasteTexts
  const dirtyPastes = useRef(new Set<number>())
  const [openPaste, setOpenPaste] = useState<number | null>(null)
  const pasteBody = useKoma((s) => s.ui.pasteBody)
  const consumePasteBody = useKoma((s) => s.consumePasteBody)
  const sessionId = useKoma((s) => s.session.id)
  const diagramChatQueue = useKoma((s) => s.ui.diagramChatQueue)
  const consumeDiagramChatQueue = useKoma((s) => s.consumeDiagramChatQueue)
  const [dragOver, setDragOver] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const overlayRef = useRef<HTMLDivElement>(null)
  // Tokens this session has inserted via the omnisearch picker (bare, e.g.
  // "@downloads/file.pdf" or "@[1]downloads/file.pdf" — never with the
  // trailing space) — read by the overlay renderer + atomic chip-delete below
  // to decide which whitespace-delimited draft tokens are chip-eligible.
  // Add-only: stale entries that no longer appear in the text are harmless,
  // this is only ever membership-tested, never iterated positionally.
  const pickedTokensRef = useRef<Set<string>>(new Set())
  // Mascot swap-on-send: bumped once per submit, telling CatMascot to pick a
  // different random cat. Otherwise it just keeps looping the current one.
  const [mascotSwap, setMascotSwap] = useState(0)
  // Thinking-bubble word, re-randomized every 1s while `working` is true (see
  // effect below). Empty when idle; the bubble itself is hidden via
  // `working` so a stale word never flashes on the next turn.
  const [thinkingWord, setThinkingWord] = useState('')

  // Tracks the textarea's width across ResizeObserver/window-resize firings so
  // the reflow handler below only re-autosizes when the width actually
  // changed (a height-only firing — e.g. the autosize effect's own mutation
  // observed indirectly via the parent wrapper — would otherwise loop).
  const lastWidthRef = useRef(0)

  // Auto-grow the textarea to fit its content, up to a cap (then it scrolls).
  // Shared by the [input] effect (every keystroke / programmatic change) and
  // the reflow handler below (width changes with the SAME text).
  const autosizeTextarea = () => {
    const ta = textareaRef.current
    if (!ta) return
    ta.style.height = 'auto'
    ta.style.height = `${Math.min(ta.scrollHeight, 200)}px`
  }

  // Runs on every input change (incl. programmatic clears + omnisearch inserts).
  // Also parks the caret at the END of the text when a history recall just
  // replaced it (caretToEndRef, set by recallHistory below) — a plain typed
  // change never needs this, the browser already tracks the caret for that.
  useEffect(() => {
    const ta = textareaRef.current
    if (!ta) return
    autosizeTextarea()
    if (caretTargetRef.current !== null) {
      // Atomic chip-delete (below) requested a precise caret position — the
      // deleted token's start — rather than "end of text".
      ta.setSelectionRange(caretTargetRef.current, caretTargetRef.current)
      caretTargetRef.current = null
    } else if (caretToEndRef.current) {
      ta.setSelectionRange(ta.value.length, ta.value.length)
      caretToEndRef.current = false
    }
    // Height just changed (auto-grow above); keep the chip overlay's scroll
    // glued to the textarea (rAF catches post-keystroke caret auto-scroll).
    syncOverlayScrollSoon()
  }, [input])

  // Keep the composer correct across REFLOWS — not just keystrokes (the
  // [input] effect above). A width change (window resize, sidebar
  // collapse/expand) reflows the textarea's wrapped-line layout, which changes
  // its natural `scrollHeight` for the SAME text — so the height set by the
  // last autosize goes stale until the next keystroke recomputes it. A
  // `ResizeObserver` on the PARENT wrapper (not the textarea itself — observing
  // the element whose height this handler mutates would invite observer
  // loops) catches that; the `window resize` listener stays as a fallback for
  // environments where the observer doesn't fire. Both funnel through the same
  // width-gated handler so a height-only firing (e.g. the autosize mutation
  // itself, reflected onto the wrapper) is a no-op. Fires once on mount too,
  // in case anything sizes late.
  useEffect(() => {
    const wrapper = textareaRef.current?.parentElement ?? null
    const handleReflow = () => {
      const width = wrapper?.clientWidth ?? 0
      if (width === lastWidthRef.current) return
      lastWidthRef.current = width
      autosizeTextarea()
      syncOverlayScrollSoon()
    }
    handleReflow()
    const observer = wrapper ? new ResizeObserver(handleReflow) : null
    if (wrapper) observer?.observe(wrapper)
    window.addEventListener('resize', handleReflow)
    return () => {
      observer?.disconnect()
      window.removeEventListener('resize', handleReflow)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Thinking-bubble: while working, pick a fresh word immediately, then
  // re-randomize every 1000ms (avoiding an immediate repeat) until working
  // flips false — at which point the interval is torn down and the bubble
  // fades out via its own `working`-gated styling. Also cleaned up on unmount.
  useEffect(() => {
    if (!working) return
    const pick = (current: string) => {
      if (wanderWords.length <= 1) return wanderWords[0] ?? ''
      let next = current
      while (next === current) {
        next = wanderWords[Math.floor(Math.random() * wanderWords.length)]
      }
      return next
    }
    setThinkingWord((prev) => pick(prev))
    const id = window.setInterval(() => {
      setThinkingWord((prev) => pick(prev))
    }, 1000)
    return () => window.clearInterval(id)
  }, [working])

  // Consume one-shot omnisearch-pick signals: append the picked path into
  // the draft text (not an attachment — the daemon's ingest is image-only,
  // see attachFiles below), then ack so it doesn't re-fire on rerender.
  useEffect(() => {
    if (composerInsert === null) return
    // Record the bare token (no trailing space) as chip-eligible for the
    // overlay + atomic-delete below — covers both the `@label ` common case
    // and the raw-path fallback (empty label), so a fallback insert still
    // renders/deletes as a single unit even though it has no `@` prefix.
    const token = composerInsert.trimEnd()
    if (token) pickedTokensRef.current.add(token)
    setInput((prev) => (prev.length > 0 ? `${prev} ${composerInsert}` : composerInsert))
    consumeComposerInsert()
  }, [composerInsert, consumeComposerInsert])

  // A diagram reference is a drawing chip. The Mermaid stays on the chip until
  // send, which is the text the model actually receives.
  useEffect(() => {
    if (!diagramChatQueue.length) return
    setDiagramChips((prev) => [
      ...prev,
      ...diagramChatQueue.map((item) => ({ id: mintDiagramChipId(), ...item })),
    ])
    consumeDiagramChatQueue()
    textareaRef.current?.focus()
  }, [diagramChatQueue, consumeDiagramChatQueue])

  useEffect(() => {
    if (!pasteBody) return
    if (!dirtyPastes.current.has(pasteBody.markerN)) {
      setPasteTexts((prev) => ({ ...prev, [pasteBody.markerN]: pasteBody.text }))
    }
    consumePasteBody()
  }, [pasteBody, consumePasteBody])

  // Consume one-shot rewind refills: REPLACE the draft with the rewound
  // message's text (unlike composerInsert, which appends) so the user can edit
  // and resend it. Ack immediately so it doesn't re-fire on rerender.
  useEffect(() => {
    if (composerRefill === null) return
    const draft = chipsFromMessage(composerRefill)
    setDiagramChips(draft.chips)
    setLocalPastes(draft.pastes)
    setInput(draft.prose)
    consumeComposerRefill()
  }, [composerRefill, consumeComposerRefill])

  // Paste chips are session-local. A new session must not reuse marker #1's
  // edited body, or bind a new paste onto the previous session's queue.
  useEffect(() => {
    setPasteTexts({})
    setOpenPaste(null)
    setLocalPastes([])
    dirtyPastes.current.clear()
    attachQueue.current = []
    submitArmed.current = false
    seenPasteMarkers.current = new Set(
      useKoma.getState().session.attachments.filter((item) => item.kind === 'pasted_text').map((item) => item.markerN),
    )
  }, [sessionId])

  // Steer cap: the daemon queues at most 5 pending mid-turn submits; the 6th is
  // dropped host-side with a toast, so gate send at the cap.
  const atSteerCap = pendingSteer.length >= 5

  // Keep list selection in range as the queue shrinks (drain / remove / clear).
  useEffect(() => {
    if (pendingSteer.length === 0) {
      setSteerSel(0)
      setSteerFocus(false)
      return
    }
    setSteerSel((s) => Math.min(s, pendingSteer.length - 1))
  }, [pendingSteer.length])

  const editSteerAt = (index: number) => {
    const text = pendingSteer[index]
    if (text === undefined) return
    // GUI composer does not reconcile InputChanged — refill locally, then tell
    // the daemon to drop that queue slot (mirrors TUI EditSteer).
    refillComposer(text)
    req({ r: 'EditSteer', index })
    setSteerFocus(false)
    textareaRef.current?.focus()
  }

  const removeSteerAt = (index: number) => {
    if (index < 0 || index >= pendingSteer.length) return
    req({ r: 'RemoveSteer', index })
  }

  // Up/Down composer history recall (client-side only, no daemon round-trip —
  // mirrors the TUI's hist_idx + input_stash, state/runtime.rs:887-906).
  // histIdxRef is -1 when not currently recalling; stashRef holds the
  // in-progress draft, restored once the user walks back past the newest
  // recalled entry. Both reset on any user edit (onChange) or send (submit).
  const histIdxRef = useRef(-1)
  const stashRef = useRef('')
  const stashChipsRef = useRef<DiagramChip[]>([])
  const stashPastesRef = useRef<LocalPaste[]>([])
  // Flags the [input] auto-grow effect above to also park the caret at the end
  // of the text a recall just injected (a plain typed change never needs this).
  const caretToEndRef = useRef(false)
  // Set by the atomic chip-delete handler (below) to park the caret at a
  // precise offset — the deleted token's start — after the [input] effect's
  // setInput-triggered rerender. Parallels caretToEndRef, but for an exact
  // position instead of "end of text"; checked first since it's the more
  // specific request.
  const caretTargetRef = useRef<number | null>(null)

  const resetHistory = () => {
    histIdxRef.current = -1
    stashRef.current = ''
    stashChipsRef.current = []
    stashPastesRef.current = []
  }

  const showRecalled = (text: string) => {
    const draft = chipsFromMessage(text)
    // The recalled draft replaces chips that were still waiting for a marker.
    // Cancel those rows so the late number is dropped instead of appearing
    // beside the recalled message.
    for (const row of attachQueue.current) {
      if (row.markerN == null) row.cancelled = true
    }
    submitArmed.current = false
    setDiagramChips(draft.chips)
    setLocalPastes(draft.pastes)
    setInput(draft.prose)
  }

  // Keep the chip overlay's scroll position glued to the textarea's — has to
  // track both user scrolling (wheel/keys inside a >200px-tall draft, once
  // the textarea itself scrolls internally) and programmatic height changes
  // (the autosize effect above). Also re-sync on the next frame: after a
  // keystroke the browser may auto-scroll the caret into view *after* our
  // layout effect, and without a follow-up the overlay lags one paint.
  const syncOverlayScroll = () => {
    const overlay = overlayRef.current
    const ta = textareaRef.current
    if (!overlay || !ta) return
    overlay.scrollTop = ta.scrollTop
    overlay.scrollLeft = ta.scrollLeft
  }
  const syncOverlayScrollSoon = () => {
    syncOverlayScroll()
    requestAnimationFrame(syncOverlayScroll)
  }

  // Read straight off the store (no subscription — this only runs on an
  // Up/Down keypress, not every render) for user-authored, plain messages:
  // role==='user', no `kind` (excludes 'shell'/'bashNudge' — recalling a
  // "$ cmd\noutput" blob into the composer is garbage, a deliberate deviation
  // from the TUI which has no such rows), non-empty content. Oldest-first.
  const recallCandidates = () =>
    useKoma
      .getState()
      .session.messages.filter((m) => m.role === 'user' && !m.kind && m.content.trim() !== '')

  const toastError = (text: string) => {
    const id = useKoma.getState().ui.toastSeq + 1
    useKoma.setState((s) => ({
      ui: { ...s.ui, toastSeq: id, toast: { id, text, kind: 'error' } },
    }))
  }

  const submit = () => {
    // The chip is editable immediately. Send waits until the snapshot has
    // assigned `[Pasted Text #N]`, then flushes this same draft.
    if (attachQueue.current.some((row) => !row.cancelled && row.markerN == null)) {
      submitArmed.current = true
      return
    }
    const prose = input.trim()
    const mermaid = diagramChips.map((chip) => chip.mermaid).join('\n\n')
    const locals = localPastesRef.current
    const linked = new Set(locals.flatMap((item) => (item.markerN != null ? [item.markerN] : [])))
    const recalled = locals.filter((item) => item.markerN == null)
    const staged = attachments.filter((item) => item.kind === 'pasted_text' && !linked.has(item.markerN))
    const fences = recalled.map((item) => formatPasteFence(item)).join('\n\n')
    const markers = [
      ...locals.flatMap((item) => (item.markerN != null ? [pasteMarker(item.markerN)] : [])),
      ...staged.map((item) => pasteMarker(item.markerN)),
    ]
    const text = [prose, mermaid, fences, markers.join(' ')].filter(Boolean).join('\n\n')
    const stagedPaste = staged.length > 0 || locals.some((item) => item.markerN != null)
    if (!text && !stagedPaste) return
    const bodies = [
      ...locals.map((item) => item.text),
      ...staged.flatMap((item) => {
        const body = pasteTextsRef.current[item.markerN]
        return body == null ? [] : [body]
      }),
    ]
    if (bodies.some((body) => pasteByteLength(body) > PASTE_SOFT_MAX_BYTES)) {
      toastError('Paste is larger than 2 MB')
      return
    }
    for (const item of locals) {
      if (item.markerN == null) continue
      req({ r: 'UpdatePaste', markerN: item.markerN, text: item.text })
    }
    for (const markerN of dirtyPastes.current) {
      const body = pasteTextsRef.current[markerN]
      if (body == null) continue
      if (!staged.some((item) => item.markerN === markerN)) continue
      req({ r: 'UpdatePaste', markerN, text: body })
    }
    dirtyPastes.current.clear()
    // While working, a submit is QUEUED daemon-side as a steer (not a new turn);
    // block it at the cap so we don't fire a request the daemon will just drop.
    if (atSteerCap) return
    resetHistory()
    // Staged rewind (edit pencil): fire RewindTo FIRST so the daemon aborts the
    // in-flight turn + truncates messages.json to before the edited message, THEN
    // Submit carries the edited text as the fresh turn. The single ordered IPC
    // channel guarantees RewindTo (abort + truncate) runs before Submit starts.
    if (pendingRewindIndex !== null) {
      req({ r: 'RewindTo', index: pendingRewindIndex })
      clearRewind()
    }
    // `!<cmd>` composer shell shortcut (TUI parity, controller/input/chat.rs:
    // 418-442): route to a no-model-round-trip shell run instead of a chat
    // submit — but ONLY while idle and with no staged image attachment (an
    // attachment makes no sense on a shell line; fall through to a normal
    // Submit instead, same as an empty `!cmd`). Deliberate deviation from the
    // TUI: it no-ops a `!` line while busy, but here we let it fall through to
    // a normal Submit so it queues as a steer like any other composer send,
    // rather than silently dropping the keystroke.
    if (!working && attachments.length === 0 && diagramChips.length === 0 && localPastesRef.current.length === 0 && text.startsWith('!')) {
      const cmd = text.slice(1).trim()
      if (cmd) {
        req({ r: 'Shell', cmd })
        setInput('')
        setMascotSwap((t) => t + 1)
        requestScrollBottom()
        return
      }
    }
    req({ r: 'Submit', text })
    setInput('')
    setDiagramChips([])
    setLocalPastes([])
    setOpenPaste(null)
    // Keep cancelled rows that are still waiting for a marker so the late
    // snapshot can drop the chip the user already removed.
    attachQueue.current = attachQueue.current.filter((row) => row.cancelled && row.markerN == null)
    // Swap the mascot to a new random cat on every send.
    setMascotSwap((t) => t + 1)
    // Force the transcript back to the bottom on send (re-engages the W4
    // scroll-stick even if the user had scrolled up).
    requestScrollBottom()
  }
  submitRef.current = submit

  // Snapshot assigned marker numbers. Bind them onto the chips that are still
  // waiting, or drop a chip the user removed before the number came back.
  // A send that landed during the wait flushes once every live row is bound.
  useEffect(() => {
    const assigned = assignFreshPasteMarkers(attachQueue.current, seenPasteMarkers.current, attachments)
    if (!assigned.length) return
    const removals = assigned.filter((row) => row.cancelled && row.markerN != null)
    const keeps = assigned.filter((row) => !row.cancelled && row.markerN != null)
    for (const row of removals) {
      if (row.markerN == null) continue
      req({ r: 'RemoveAttachment', markerN: row.markerN, kind: 'pasted_text' })
    }
    if (keeps.length) {
      setLocalPastes((prev) => prev.map((item) => {
        const hit = keeps.find((row) => row.id === item.id)
        return hit && hit.markerN != null ? { ...item, markerN: hit.markerN, n: hit.markerN } : item
      }))
    }
  }, [attachments, req])

  // Flush a send that happened while the paste marker was still in flight.
  // Wait until the chip state shows the marker so the body edit is what we save.
  useEffect(() => {
    if (!submitArmed.current) return
    if (attachQueue.current.some((row) => !row.cancelled && row.markerN == null)) return
    const unmarked = localPastes.some((item) => {
      const row = attachQueue.current.find((queued) => queued.id === item.id)
      return !!row && !row.cancelled && row.markerN != null && item.markerN == null
    })
    if (unmarked) return
    submitArmed.current = false
    submitRef.current()
  }, [localPastes, attachments])

  // If the daemon never stages the paste, stop blocking send and keep the
  // draft text. The queue row is dropped so a later, different paste is not
  // paired with this one.
  useEffect(() => {
    const waiting = localPastes.some((item) =>
      attachQueue.current.some((row) => row.id === item.id && !row.cancelled && row.markerN == null),
    )
    if (!waiting) return
    const timer = window.setTimeout(() => {
      const stuck = new Set(
        attachQueue.current.filter((row) => row.markerN == null && !row.cancelled).map((row) => row.id),
      )
      if (!stuck.size) return
      attachQueue.current = attachQueue.current.filter((row) => !stuck.has(row.id))
      if (!submitArmed.current) return
      submitArmed.current = false
      const id = useKoma.getState().ui.toastSeq + 1
      useKoma.setState((s) => ({
        ui: { ...s.ui, toastSeq: id, toast: { id, text: 'Pasted text stayed in the draft. Press send again.', kind: 'error' } },
      }))
    }, 8000)
    return () => window.clearTimeout(timer)
  }, [localPastes])

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    // Follow-ups list focus: when the queue owns keys, Enter edits, arrows move,
    // Delete removes, Esc unfocuses (does not clear). Ctrl+X clears all below.
    if (steerFocus && pendingSteer.length > 0) {
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setSteerSel((s) => Math.max(0, s - 1))
        return
      }
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setSteerSel((s) => {
          if (s + 1 < pendingSteer.length) return s + 1
          setSteerFocus(false)
          return s
        })
        return
      }
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault()
        editSteerAt(steerSel)
        return
      }
      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault()
        removeSteerAt(steerSel)
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        setSteerFocus(false)
        return
      }
      // Any printable char drops focus and falls through to the textarea.
      if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
        setSteerFocus(false)
      }
    }

    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      submit()
      return
    }
    // Ctrl/Cmd+X clears every queued follow-up (TUI Ctrl+X parity).
    if ((e.ctrlKey || e.metaKey) && (e.key === 'x' || e.key === 'X') && pendingSteer.length > 0) {
      e.preventDefault()
      req({ r: 'CancelSteers' })
      setSteerFocus(false)
      return
    }
    // Atomic chip delete: Backspace/Delete next to (or inside) a chip-eligible
    // `@label` range removes the WHOLE range in one keystroke instead of
    // eating it character by character. Gated on isComposing so an IME
    // candidate-confirm Backspace never gets hijacked. Non-collapsed
    // selections (start !== end) fall through to native behavior untouched.
    if ((e.key === 'Backspace' || e.key === 'Delete') && !e.nativeEvent.isComposing) {
      const ta = e.currentTarget
      const start = ta.selectionStart ?? 0
      const end = ta.selectionEnd ?? 0
      if (start === end) {
        const text = ta.value
        const ranges = findChipRanges(text, pickedTokensRef.current)
        const span =
          e.key === 'Backspace' ? chipRangeForBackspace(ranges, start) : chipRangeForDelete(ranges, start)
        if (span) {
          let [spanStart, spanEnd] = span
          // Eat exactly one trailing LITERAL SPACE along with the chip (the
          // space OmniSearchPalette always inserts after it) — NOT any
          // whitespace char, since a chip sitting at end-of-line in a
          // multi-line draft would otherwise eat the newline and merge the
          // next line up.
          if (text[spanEnd] === ' ') spanEnd += 1
          e.preventDefault()
          caretTargetRef.current = spanStart
          setInput(text.slice(0, spanStart) + text.slice(spanEnd))
          return
        }
      }
    }
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      // Omnisearch owns Up/Down for its own result-list navigation while open.
      if (omnisearchOpen) return
      const ta = e.currentTarget
      const firstLine = !ta.value.slice(0, ta.selectionStart ?? 0).includes('\n')
      const lastLine = !ta.value.slice(ta.selectionEnd ?? ta.value.length).includes('\n')
      // From the first composer line, ↑ enters the follow-ups list when non-empty.
      if (e.key === 'ArrowUp' && firstLine && pendingSteer.length > 0) {
        e.preventDefault()
        setSteerFocus(true)
        setSteerSel(pendingSteer.length - 1)
        return
      }
      if (e.key === 'ArrowUp' && firstLine) {
        const history = recallCandidates()
        if (histIdxRef.current === -1) {
          if (history.length === 0) return
          stashRef.current = input
          stashChipsRef.current = diagramChipsRef.current
          stashPastesRef.current = localPastesRef.current
          histIdxRef.current = history.length - 1
        } else if (histIdxRef.current > 0) {
          histIdxRef.current -= 1
        } else {
          return // already at the oldest entry — nothing further to recall
        }
        e.preventDefault()
        caretToEndRef.current = true
        showRecalled(history[histIdxRef.current].content)
      } else if (e.key === 'ArrowDown' && lastLine) {
        if (histIdxRef.current === -1) return // nothing recalled yet
        const history = recallCandidates()
        if (histIdxRef.current < history.length - 1) {
          histIdxRef.current += 1
          e.preventDefault()
          caretToEndRef.current = true
          showRecalled(history[histIdxRef.current].content)
        } else {
          // Walked past the newest recalled entry — restore the stashed draft.
          histIdxRef.current = -1
          e.preventDefault()
          caretToEndRef.current = true
          setDiagramChips(stashChipsRef.current)
          setLocalPastes(stashPastesRef.current)
          setInput(stashRef.current)
        }
      }
    }
  }

  // Draft change: clearing the composer to empty CANCELS a staged rewind (edit
  // pencil) — the user backed out, so the next send must NOT truncate. Only a
  // user edit fires onChange; programmatic refills (rewind/omnisearch/history
  // recall) go through setInput directly, so staging a rewind never
  // self-cancels here. A user edit also resets any in-progress history walk.
  const onChange = (e: ChangeEvent<HTMLTextAreaElement>) => {
    const val = e.target.value
    setInput(val)
    if (val.trim() === '' && pendingRewindIndex !== null) clearRewind()
    resetHistory()
  }

  const attachFiles = async (files: FileList | File[]) => {
    for (const file of Array.from(files)) {
      // Images only — the daemon's attachment ingest (Paste{path}) only
      // ingests image extensions; a non-image file falls through to
      // inserting its raw path into the shared composer buffer, silently
      // corrupting the session. Silently skip non-image files here; use
      // omnisearch to reference non-image workspace files by path instead.
      if (!file.type.startsWith('image/')) continue
      try {
        const bytesB64 = await readFileAsBase64(file)
        req({ r: 'AttachFile', name: file.name, bytesB64, mime: file.type || undefined })
      } catch {
        /* unreadable file — skip */
      }
    }
  }

  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(e.clipboardData?.files ?? [])
    if (files.length > 0) {
      e.preventDefault()
      void attachFiles(files)
      return
    }
    const raw = e.clipboardData?.getData('text/plain') ?? ''
    if (!raw) return
    const text = raw.replace(/\r\n/g, '\n').replace(/\r/g, '\n')
    if (pasteByteLength(text) > PASTE_SOFT_MAX_BYTES) {
      e.preventDefault()
      toastError('Paste is larger than 2 MB')
      return
    }
    if (!shouldCollapsePaste(text)) return
    e.preventDefault()
    const id = mintDiagramChipId()
    attachQueue.current.push({ id, markerN: null, cancelled: false })
    setLocalPastes((prev) => [...prev, { id, n: 0, path: '', text }])
    req({ r: 'AttachPaste', text })
  }

  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault()
    setDragOver(false)
    // Coding tree path reference (not a file upload).
    const coding = readCodingPathDragData(e.dataTransfer)
    if (coding) {
      useKoma.getState().putCodingPathInChat(coding.root, coding.path, {
        isDir: coding.isDir,
      })
      return
    }
    if (e.dataTransfer.files.length > 0) void attachFiles(e.dataTransfer.files)
  }

  const onDragOver = (e: DragEvent<HTMLDivElement>) => {
    // Accept coding-tree path drags and external image files.
    e.preventDefault()
    setDragOver(true)
    try {
      e.dataTransfer.dropEffect = 'copy'
    } catch {
      /* ignore */
    }
  }

  const onDragLeave = () => setDragOver(false)

  const onFilePicked = (e: ChangeEvent<HTMLInputElement>) => {
    if (e.target.files && e.target.files.length > 0) void attachFiles(e.target.files)
    e.target.value = ''
  }

  const removeAttachment = (markerN: number, kind: 'image' | 'file' | 'pasted_text') => {
    req({ r: 'RemoveAttachment', markerN, kind })
    if (kind === 'pasted_text') {
      dirtyPastes.current.delete(markerN)
      setOpenPaste((current) => (current === markerN ? null : current))
    }
  }

  const editPaste = (markerN: number, text: string) => {
    dirtyPastes.current.add(markerN)
    setPasteTexts((prev) => ({ ...prev, [markerN]: text }))
  }

  const removeDiagramChip = (id: string) => {
    const next = diagramChips.filter((chip) => chip.id !== id)
    setDiagramChips(next)
    if (next.length === 0 && input.trim() === '' && pendingRewindIndex !== null) clearRewind()
  }

  const canSend = (input.trim() !== '' || diagramChips.length > 0 || localPastes.length > 0 || attachments.some((item) => item.kind === 'pasted_text')) && !atSteerCap

  return (
    // claude.ai-style composer pinned at the bottom: a single rounded card
    // (textarea on top, an action bar below) that grows with its content. Drag
    // a file anywhere over the card to attach; the card rings on drag-over.
    <div className="px-2 pb-3 pt-1" data-tour="composer">
      {/* Follow-ups queue: submits made while the turn is cooking are queued
          daemon-side (cap 5). Selectable list — click or ↑ from composer. */}
      {pendingSteer.length > 0 && (
        <div
          className={`mb-1.5 flex flex-col gap-1 rounded-xl border bg-koma-panel px-2.5 py-2 ${
            steerFocus ? 'border-koma-accent' : 'border-koma-border'
          }`}
        >
          <div className="flex items-center gap-1.5 text-[11px] text-koma-dim">
            <Layers size={12} className="flex-none" />
            <span>
              follow-ups {pendingSteer.length}/5
            </span>
            <button
              onClick={() => {
                req({ r: 'CancelSteers' })
                setSteerFocus(false)
              }}
              aria-label="Clear all follow-ups"
              title="Clear all follow-ups"
              className="ml-auto flex-none opacity-60 transition-opacity hover:text-koma-fg hover:opacity-100"
            >
              <X size={12} />
            </button>
          </div>
          <div className="flex flex-col gap-0.5">
            {pendingSteer.map((s, i) => {
              const selected = steerFocus && i === steerSel
              const oneLine = steerPreview(s) || s.replace(/\s+/g, ' ').trim()
              return (
                <div
                  key={i}
                  role="button"
                  tabIndex={0}
                  onClick={() => {
                    setSteerFocus(true)
                    setSteerSel(i)
                    editSteerAt(i)
                  }}
                  onKeyDown={(ev) => {
                    if (ev.key === 'Enter' || ev.key === ' ') {
                      ev.preventDefault()
                      setSteerFocus(true)
                      setSteerSel(i)
                      editSteerAt(i)
                    }
                  }}
                  className={`flex cursor-pointer items-center gap-1.5 rounded-md px-1 py-0.5 text-[11.5px] ${
                    selected
                      ? 'bg-koma-hover text-koma-accent'
                      : 'text-koma-fg opacity-80 hover:bg-koma-hover/60'
                  }`}
                  title="Click to edit in composer"
                >
                  <span className="flex-none text-[10px]">{selected ? '●' : '○'}</span>
                  <span className="truncate">{oneLine}</span>
                  <button
                    type="button"
                    onClick={(ev) => {
                      ev.stopPropagation()
                      removeSteerAt(i)
                    }}
                    aria-label="Remove follow-up"
                    title="Remove"
                    className="ml-auto flex-none opacity-50 transition-opacity hover:opacity-100"
                  >
                    <X size={11} />
                  </button>
                </div>
              )
            })}
          </div>
          <div className="text-[10px] text-koma-dim">
            {steerFocus
              ? 'enter edit · ↑↓ select · del remove · esc unfocus · ctrl+x clear'
              : '↑ or click to select · ctrl+x clear all'}
          </div>
        </div>
      )}
      <div
        onDrop={onDrop}
        onDragOver={onDragOver}
        onDragLeave={onDragLeave}
        className={`relative flex flex-col gap-2 rounded-2xl border bg-koma-panel px-3 py-2.5 shadow-sm transition-colors @max-xs/chat:gap-1.5 @max-xs/chat:rounded-xl @max-xs/chat:px-2 @max-xs/chat:py-2 @max-[14rem]/chat:px-1.5 @max-[14rem]/chat:py-1.5 ${
          dragOver ? 'border-koma-accent bg-koma-hover' : 'border-koma-border'
        }`}
      >
        {/* Persistent mascot: a small always-looping cat perched on the card's
            top-right corner. Purely decorative — not gated on `working` — and
            swaps to a different random cat on every send (see submit above). */}
        <CatMascot swapTrigger={mascotSwap} />

        {/* Thinking bubble: floats ABOVE the cat (not beside it), only while
            `working`. The cat sits at -top-3/right-3 (h-12), so its top edge is
            12px above the card and its box is 48px tall; anchoring the bubble at
            -top-11 (-44px) puts its bottom edge ~8px above the cat's top edge (a
            small gap), while `right-3` matches the cat's right edge exactly.
            Only `right` is set (no `left`), so the pill still grows
            leftward/from-the-right as its word content needs. The nearest
            overflow-hidden ancestor is the shell's main content region
            (routes/index.tsx `top-8 flex overflow-hidden`), which spans nearly
            the full window height above the composer — the extra ~32px of
            poke-up room this needs (vs. the cat's 12px) stays well inside that
            box in any normal window size, so no portal is needed here (unlike
            e.g. Select/Combobox menus, which portal to <body> because they can
            open far down an overflow:auto list). Kept mounted (not conditionally
            rendered) so opacity/translate can transition instead of popping. */}
        <div
          className={`pointer-events-none absolute -top-11 right-3 z-10 transition-all duration-300 ${
            working ? 'translate-y-0 opacity-100' : 'translate-y-1 opacity-0'
          }`}
          aria-hidden="true"
        >
          <span className="whitespace-nowrap rounded-full border border-koma-border bg-koma-panel2 px-2.5 py-1 text-[11px] text-koma-dim shadow-sm">
            {thinkingWord.toLowerCase()}…
          </span>
        </div>

        {diagramChips.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {diagramChips.map((chip) => (
              <span
                key={chip.id}
                className="flex items-center gap-1.5 rounded-lg border border-koma-border bg-koma-panel2 py-1 pl-1 pr-2 text-[11px] text-koma-fg"
              >
                <span className="h-8 w-12 flex-none overflow-hidden rounded border border-koma-border bg-koma-bg">
                  <DiagramSketch doc={chip.doc} />
                </span>
                <span className="max-w-[140px] truncate">{chip.title}</span>
                <button
                  type="button"
                  onClick={() => removeDiagramChip(chip.id)}
                  aria-label={`Remove ${chip.title}`}
                  className="flex-none opacity-60 transition-opacity hover:opacity-100"
                >
                  <X size={11} />
                </button>
              </span>
            ))}
          </div>
        )}

        {attachments.some((item) => item.kind !== 'pasted_text' || !localPastes.some((chip) => chip.markerN === item.markerN)) && (
          <div className="flex flex-col gap-1">
            <div className="flex flex-wrap gap-1">
              {attachments.filter((item) => item.kind !== 'pasted_text' || !localPastes.some((chip) => chip.markerN === item.markerN)).map((a) => (
                <span
                  key={`${a.kind}:${a.markerN}`}
                  className="flex items-center gap-1 rounded-lg border border-koma-border bg-koma-panel2 px-2 py-1 text-[11px] text-koma-fg opacity-90"
                >
                  {a.kind === 'pasted_text' ? (
                    <button
                      type="button"
                      className="max-w-[180px] truncate text-left"
                      onClick={() => {
                        setOpenPaste((current) => (current === a.markerN ? null : a.markerN))
                        if (pasteTextsRef.current[a.markerN] == null) req({ r: 'ReadPaste', markerN: a.markerN })
                      }}
                    >
                      {a.name}
                    </button>
                  ) : (
                    <span className="max-w-[140px] truncate">{a.name}</span>
                  )}
                  <button
                    onClick={() => removeAttachment(a.markerN, a.kind)}
                    aria-label={`Remove ${a.name}`}
                    className="flex-none opacity-60 transition-opacity hover:opacity-100"
                  >
                    <X size={11} />
                  </button>
                </span>
              ))}
            </div>
            {attachments
              .filter((item) => item.kind === 'pasted_text' && item.markerN === openPaste && !localPastes.some((chip) => chip.markerN === item.markerN))
              .map((item) => (
                pasteTexts[item.markerN] == null ? (
                  <p key={`edit-${item.markerN}`} className="px-2 py-1.5 text-[12px] text-koma-dim">Loading pasted text…</p>
                ) : (
                  <textarea
                    key={`edit-${item.markerN}`}
                    value={pasteTexts[item.markerN]}
                    aria-label={`Edit ${item.name}`}
                    onChange={(e) => editPaste(item.markerN, e.target.value)}
                    className="max-h-40 min-h-16 w-full resize-y rounded-lg border border-koma-border bg-koma-bg px-2 py-1.5 text-[12px] text-koma-fg outline-none"
                  />
                )
              ))}
          </div>
        )}
        {localPastes.length > 0 && (
          <div className="flex flex-col gap-1">
            {localPastes.map((item) => {
              const attaching = item.markerN == null && attachQueue.current.some((row) => row.id === item.id && !row.cancelled && row.markerN == null)
              const label = item.markerN != null ? `Pasted Text #${item.markerN}` : attaching ? 'Attaching paste…' : 'Pasted text'
              return (
              <span key={item.id} className="rounded-lg border border-koma-border bg-koma-panel2 px-2 py-1 text-[11px] text-koma-fg">
                <span className="flex items-center gap-1">
                  <span className="min-w-0 flex-1 truncate">{label}</span>
                  <button
                    type="button"
                    onClick={() => {
                      const row = attachQueue.current.find((queued) => queued.id === item.id)
                      if (row && row.markerN == null) row.cancelled = true
                      else if (item.markerN != null) removeAttachment(item.markerN, 'pasted_text')
                      setLocalPastes((prev) => prev.filter((chip) => chip.id !== item.id))
                    }}
                    aria-label={`Remove ${label}`}
                    className="flex-none opacity-60 hover:opacity-100"
                  >
                    <X size={11} />
                  </button>
                </span>
                <textarea
                  value={item.text}
                  aria-label={`Edit ${label}`}
                  onChange={(e) => setLocalPastes((prev) => prev.map((chip) => (chip.id === item.id ? { ...chip, text: e.target.value } : chip)))}
                  className="mt-1 max-h-40 min-h-16 w-full resize-y rounded border border-koma-border bg-koma-bg px-2 py-1 text-[12px] outline-none"
                />
              </span>
              )
            })}
          </div>
        )}

        {/* Wraps ONLY the textarea: a `relative z-0` positioning root for the
            chip overlay (absolute inset-0 behind it) — isolated as its own
            z-stacking context (explicit z-0 on a positioned element) so the
            overlay/textarea's internal z-0/z-10 ordering never competes with
            the card-level mascot/thinking-bubble (both z-10) above. */}
        <div className="relative z-0">
          {/* Chip overlay: mirrors the textarea's text behind it (see
              renderComposerOverlay above), painting chip-eligible `@label`
              tokens as tinted pills. pointer-events-none so it never steals
              clicks/caret placement from the (visually transparent, but very
              much alive) textarea layered on top of it.
              Same field class + overflow-y:auto + stable gutter as the
              textarea so wrap width and line boxes stay pixel-aligned once
              the draft hits max-height and a scrollbar appears. */}
          <div
            ref={overlayRef}
            aria-hidden="true"
            className={`pointer-events-none absolute inset-0 z-0 overflow-x-hidden overflow-y-auto text-koma-fg [scrollbar-gutter:stable] ${COMPOSER_FIELD_CLASS}`}
          >
            {renderComposerOverlay(input, pickedTokensRef.current)}
          </div>
          <textarea
            ref={textareaRef}
            value={input}
            onChange={onChange}
            onKeyDown={onKeyDown}
            onPaste={onPaste}
            onScroll={syncOverlayScroll}
            placeholder="Message koma…"
            rows={1}
            className={`relative z-10 max-h-[200px] min-h-[22px] resize-none overflow-x-hidden overflow-y-auto bg-transparent outline-none [scrollbar-gutter:stable] caret-koma-fg placeholder:text-koma-fg placeholder:opacity-40 ${COMPOSER_FIELD_CLASS} ${
              input === '' ? 'text-koma-fg' : 'text-transparent'
            }`}
          />
        </div>

        <div className="flex min-w-0 items-center justify-between gap-1">
          <div className="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden @max-xs/chat:gap-0">
            <input
              ref={fileInputRef}
              type="file"
              accept="image/*"
              multiple
              className="hidden"
              onChange={onFilePicked}
            />
            <button
              onClick={() => fileInputRef.current?.click()}
              aria-label="Attach file"
              title="Attach file"
              className="flex h-8 w-8 flex-none items-center justify-center rounded-lg text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100 @max-[14rem]/chat:h-7 @max-[14rem]/chat:w-7"
            >
              <Paperclip size={16} />
            </button>
            <button
              onClick={openOmniSearch}
              aria-label="Search workspace files"
              title="Search workspace files"
              className="flex h-8 w-8 flex-none items-center justify-center rounded-lg text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100 @max-[14rem]/chat:h-7 @max-[14rem]/chat:w-7"
            >
              <Search size={16} />
            </button>
            {/* Session model quick-picker — compact, drops UP above the composer. */}
            <ModelPicker />
            {/* Reasoning effort (TUI /effort parity) — compact, drops UP above the composer. */}
            <EffortPicker />
            {/* Agent mode (Auto/Plan/Normal) — compact, drops UP above the composer. */}
            <ModeSelector />
          </div>

          <div className="flex flex-none items-center gap-1.5 @max-xs/chat:gap-1">
            {/* STOP is a SEPARATE control from send (not a morph): while the turn
                runs, both are shown — send stays LIVE so a submit QUEUES as a
                steer, and stop aborts the in-flight turn (GuiReq Interrupt). */}
            {working && (
              <button
                onClick={() => req({ r: 'Interrupt' })}
                aria-label="Stop"
                title="Stop"
                className="flex h-8 w-8 flex-none items-center justify-center rounded-full border border-koma-border text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100 @max-[14rem]/chat:h-7 @max-[14rem]/chat:w-7"
              >
                <Square size={13} className="fill-current" />
              </button>
            )}
            <button
              onClick={submit}
              disabled={!canSend}
              aria-label={working ? 'Queue message' : 'Send'}
              title={
                atSteerCap
                  ? '5 pending steers max'
                  : working
                    ? 'Queue while working'
                    : 'Send'
              }
              className={`flex h-8 w-8 flex-none items-center justify-center rounded-full transition-colors @max-[14rem]/chat:h-7 @max-[14rem]/chat:w-7 ${
                canSend
                  ? 'bg-koma-accent text-koma-bg hover:opacity-90'
                  : 'bg-koma-hover text-koma-fg opacity-40'
              }`}
            >
              <ArrowUp size={16} />
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
