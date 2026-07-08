import {
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
} from 'react'
import { ArrowUp, CornerDownRight, Layers, Paperclip, Search, Square, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { ModelPicker } from './ModelPicker'
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
  // Mascot swap-on-send: bumped once per submit, telling CatMascot to pick a
  // different random cat. Otherwise it just keeps looping the current one.
  const [mascotSwap, setMascotSwap] = useState(0)
  // Thinking-bubble word, re-randomized every 1s while `working` is true (see
  // effect below). Empty when idle; the bubble itself is hidden via
  // `working` so a stale word never flashes on the next turn.
  const [thinkingWord, setThinkingWord] = useState('')

  // Auto-grow the textarea with its content, up to a cap (then it scrolls).
  // Runs on every input change (incl. programmatic clears + omnisearch inserts).
  useEffect(() => {
    const ta = textareaRef.current
    if (!ta) return
    ta.style.height = 'auto'
    ta.style.height = `${Math.min(ta.scrollHeight, 200)}px`
  }, [input])

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

  const submit = () => {
    const text = input.trim()
    if (!text) return
    // While working, a submit is QUEUED daemon-side as a steer (not a new turn);
    // block it at the cap so we don't fire a request the daemon will just drop.
    if (atSteerCap) return
    // Staged rewind (edit pencil): fire RewindTo FIRST so the daemon aborts the
    // in-flight turn + truncates messages.json to before the edited message, THEN
    // Submit carries the edited text as the fresh turn. The single ordered IPC
    // channel guarantees RewindTo (abort + truncate) runs before Submit starts.
    if (pendingRewindIndex !== null) {
      req({ r: 'RewindTo', index: pendingRewindIndex })
      clearRewind()
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
    }
  }

  // Draft change: clearing the composer to empty CANCELS a staged rewind (edit
  // pencil) — the user backed out, so the next send must NOT truncate. Only a
  // user edit fires onChange; programmatic refills (rewind/omnisearch) go through
  // setInput directly, so staging a rewind never self-cancels here.
  const onChange = (e: ChangeEvent<HTMLTextAreaElement>) => {
    const val = e.target.value
    setInput(val)
    if (val.trim() === '' && pendingRewindIndex !== null) clearRewind()
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
    <div className="px-2 pb-3 pt-1">
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

        {/* Thinking bubble: anchored to the LEFT of the cat (the cat sits at
            -top-3/right-3, h-12), only while `working`. Shares the cat's
            -top-3/h-12 box so `items-center` centers it on the cat's vertical
            midpoint without any pixel math; `right-16` leaves an ~8px gap off
            the cat's left edge (right-3 + w-12 = 60px) and the pill grows
            leftward since only `right` is anchored. The parent card has no
            overflow-hidden, so it's never clipped. Kept mounted (not
            conditionally rendered) so opacity/translate can transition instead
            of popping. */}
        <div
          className={`pointer-events-none absolute -top-3 right-16 z-10 flex h-12 items-center transition-all duration-300 ${
            working ? 'translate-x-0 opacity-100' : 'translate-x-1 opacity-0'
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

        <textarea
          ref={textareaRef}
          value={input}
          onChange={onChange}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
          placeholder="Message koma…"
          rows={1}
          className="max-h-[200px] min-h-[24px] w-full resize-none bg-transparent text-[14px] leading-relaxed text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-40"
        />

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
