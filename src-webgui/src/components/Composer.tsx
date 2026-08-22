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
import { ArrowUp, CornerDownRight, Layers, Paperclip, Search, Square, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { ModelPicker } from './ModelPicker'
import { EffortPicker } from './EffortPicker'
import { ModeSelector } from './ModeSelector'
import { CatMascot } from './CatMascot'
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

// Overlay renderer: walks the chip ranges in order, emitting the untouched
// in-between text verbatim and wrapping each range's slice in a tinted pill
// span. The plain-text pieces + pill contents concatenate back to EXACTLY
// `text` — this must stay character-identical (same glyphs, same wrapping),
// since it paints directly behind the transparent textarea and has to line
// up with the real caret/selection pixel-for-pixel.
function renderComposerOverlay(text: string, pickedTokens: Set<string>): ReactNode {
  if (text === '') return null
  const ranges = findChipRanges(text, pickedTokens)
  if (ranges.length === 0) return text
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
  const req = useKoma((s) => s.req)
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
    // glued to the textarea's own scrollTop.
    syncOverlayScroll()
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
      syncOverlayScroll()
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

  // Consume one-shot rewind refills: REPLACE the draft with the rewound
  // message's text (unlike composerInsert, which appends) so the user can edit
  // and resend it. Ack immediately so it doesn't re-fire on rerender.
  useEffect(() => {
    if (composerRefill === null) return
    setInput(composerRefill)
    consumeComposerRefill()
  }, [composerRefill, consumeComposerRefill])

  // Steer cap: the daemon queues at most 5 pending mid-turn submits; the 6th is
  // dropped host-side with a toast, so gate send at the cap.
  const atSteerCap = pendingSteer.length >= 5

  // Up/Down composer history recall (client-side only, no daemon round-trip —
  // mirrors the TUI's hist_idx + input_stash, state/runtime.rs:887-906).
  // histIdxRef is -1 when not currently recalling; stashRef holds the
  // in-progress draft, restored once the user walks back past the newest
  // recalled entry. Both reset on any user edit (onChange) or send (submit).
  const histIdxRef = useRef(-1)
  const stashRef = useRef('')
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
  }

  // Keep the chip overlay's scroll position glued to the textarea's — has to
  // track both user scrolling (wheel/keys inside a >200px-tall draft, once
  // the textarea itself scrolls internally) and programmatic height changes
  // (the autosize effect above).
  const syncOverlayScroll = () => {
    if (overlayRef.current && textareaRef.current) {
      overlayRef.current.scrollTop = textareaRef.current.scrollTop
    }
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

  const submit = () => {
    const text = input.trim()
    if (!text) return
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
    if (!working && attachments.length === 0 && text.startsWith('!')) {
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
    // Swap the mascot to a new random cat on every send.
    setMascotSwap((t) => t + 1)
    // Force the transcript back to the bottom on send (re-engages the W4
    // scroll-stick even if the user had scrolled up).
    requestScrollBottom()
  }

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      submit()
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
      if (e.key === 'ArrowUp' && firstLine) {
        const history = recallCandidates()
        if (histIdxRef.current === -1) {
          if (history.length === 0) return
          stashRef.current = input
          histIdxRef.current = history.length - 1
        } else if (histIdxRef.current > 0) {
          histIdxRef.current -= 1
        } else {
          return // already at the oldest entry — nothing further to recall
        }
        e.preventDefault()
        caretToEndRef.current = true
        setInput(history[histIdxRef.current].content)
      } else if (e.key === 'ArrowDown' && lastLine) {
        if (histIdxRef.current === -1) return // nothing recalled yet
        const history = recallCandidates()
        if (histIdxRef.current < history.length - 1) {
          histIdxRef.current += 1
          e.preventDefault()
          caretToEndRef.current = true
          setInput(history[histIdxRef.current].content)
        } else {
          // Walked past the newest recalled entry — restore the stashed draft.
          histIdxRef.current = -1
          e.preventDefault()
          caretToEndRef.current = true
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
    if (files.length === 0) return
    e.preventDefault()
    void attachFiles(files)
  }

  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault()
    setDragOver(false)
    if (e.dataTransfer.files.length > 0) void attachFiles(e.dataTransfer.files)
  }

  const onDragOver = (e: DragEvent<HTMLDivElement>) => {
    e.preventDefault()
    setDragOver(true)
  }

  const onDragLeave = () => setDragOver(false)

  const onFilePicked = (e: ChangeEvent<HTMLInputElement>) => {
    if (e.target.files && e.target.files.length > 0) void attachFiles(e.target.files)
    e.target.value = ''
  }

  const removeAttachment = (markerN: number) => {
    req({ r: 'RemoveAttachment', markerN })
  }

  const canSend = input.trim() !== '' && !atSteerCap

  return (
    // claude.ai-style composer pinned at the bottom: a single rounded card
    // (textarea on top, an action bar below) that grows with its content. Drag
    // a file anywhere over the card to attach; the card rings on drag-over.
    <div className="px-2 pb-3 pt-1" data-tour="composer">
      {/* Pending-steer queue: submits made while the turn is cooking are queued
          daemon-side (cap 5) rather than starting a new turn. Show the queued
          previews above the composer so the user knows they're stacked up. */}
      {pendingSteer.length > 0 && (
        <div className="mb-1.5 flex flex-col gap-1 rounded-xl border border-koma-border bg-koma-panel px-2.5 py-2">
          <div className="flex items-center gap-1.5 text-[11px] text-koma-dim">
            <Layers size={12} className="flex-none" />
            <span>
              Queued {pendingSteer.length}/5
            </span>
            <button
              onClick={() => req({ r: 'CancelSteers' })}
              aria-label="Clear queued messages"
              title="Clear queued messages"
              className="ml-auto flex-none opacity-60 transition-opacity hover:text-koma-fg hover:opacity-100"
            >
              <X size={12} />
            </button>
          </div>
          <div className="flex flex-col gap-0.5">
            {pendingSteer.map((s, i) => (
              <div
                key={i}
                className="flex items-center gap-1.5 text-[11.5px] text-koma-fg opacity-80"
              >
                <CornerDownRight size={11} className="flex-none text-koma-dim" />
                <span className="truncate">{s}</span>
              </div>
            ))}
          </div>
        </div>
      )}
      <div
        onDrop={onDrop}
        onDragOver={onDragOver}
        onDragLeave={onDragLeave}
        className={`relative flex flex-col gap-2 rounded-2xl border bg-koma-panel px-3 py-2.5 shadow-sm transition-colors ${
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

        {attachments.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {attachments.map((a) => (
              <span
                key={a.markerN}
                className="flex items-center gap-1 rounded-lg border border-koma-border bg-koma-panel2 px-2 py-1 text-[11px] text-koma-fg opacity-90"
              >
                <span className="max-w-[140px] truncate">{a.name}</span>
                <button
                  onClick={() => removeAttachment(a.markerN)}
                  aria-label={`Remove ${a.name}`}
                  className="flex-none opacity-60 transition-opacity hover:opacity-100"
                >
                  <X size={11} />
                </button>
              </span>
            ))}
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
              much alive) textarea layered on top of it. */}
          <div
            ref={overlayRef}
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 z-0 w-full overflow-hidden whitespace-pre-wrap break-words text-[14px] leading-relaxed text-koma-fg"
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
            className={`relative z-10 max-h-[200px] min-h-[24px] w-full resize-none bg-transparent text-[14px] leading-relaxed outline-none caret-koma-fg placeholder:text-koma-fg placeholder:opacity-40 ${
              input === '' ? 'text-koma-fg' : 'text-transparent'
            }`}
          />
        </div>

        <div className="flex items-center justify-between">
          <div className="flex items-center gap-1">
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
              className="flex h-8 w-8 flex-none items-center justify-center rounded-lg text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
            >
              <Paperclip size={16} />
            </button>
            <button
              onClick={openOmniSearch}
              aria-label="Search workspace files"
              title="Search workspace files"
              className="flex h-8 w-8 flex-none items-center justify-center rounded-lg text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
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

          <div className="flex items-center gap-2">
            {/* STOP is a SEPARATE control from send (not a morph): while the turn
                runs, both are shown — send stays LIVE so a submit QUEUES as a
                steer, and stop aborts the in-flight turn (GuiReq Interrupt). */}
            {working && (
              <button
                onClick={() => req({ r: 'Interrupt' })}
                aria-label="Stop"
                title="Stop"
                className="flex h-8 w-8 flex-none items-center justify-center rounded-full border border-koma-border text-koma-fg opacity-80 transition-colors hover:bg-koma-hover hover:opacity-100"
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
              className={`flex h-8 w-8 flex-none items-center justify-center rounded-full transition-colors ${
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
