import {
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
} from 'react'
import { ArrowUp, Paperclip, Search, Square, X } from 'lucide-react'
import { useKoma } from '../store/koma'

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
  const req = useKoma((s) => s.req)
  const openOmniSearch = useKoma((s) => s.openOmniSearch)
  const composerInsert = useKoma((s) => s.ui.composerInsert)
  const consumeComposerInsert = useKoma((s) => s.consumeComposerInsert)
  const [input, setInput] = useState('')
  const [dragOver, setDragOver] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  // Auto-grow the textarea with its content, up to a cap (then it scrolls).
  // Runs on every input change (incl. programmatic clears + omnisearch inserts).
  useEffect(() => {
    const ta = textareaRef.current
    if (!ta) return
    ta.style.height = 'auto'
    ta.style.height = `${Math.min(ta.scrollHeight, 200)}px`
  }, [input])

  // Consume one-shot omnisearch-pick signals: append the picked path into
  // the draft text (not an attachment — the daemon's ingest is image-only,
  // see attachFiles below), then ack so it doesn't re-fire on rerender.
  useEffect(() => {
    if (composerInsert === null) return
    setInput((prev) => (prev.length > 0 ? `${prev} ${composerInsert}` : composerInsert))
    consumeComposerInsert()
  }, [composerInsert, consumeComposerInsert])

  const submit = () => {
    const text = input.trim()
    if (!text) return
    req({ r: 'Submit', text })
    setInput('')
  }

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      submit()
    }
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

  const canSend = input.trim() !== ''

  return (
    // claude.ai-style composer pinned at the bottom: a single rounded card
    // (textarea on top, an action bar below) that grows with its content. Drag
    // a file anywhere over the card to attach; the card rings on drag-over.
    <div className="px-2 pb-3 pt-1">
      <div
        onDrop={onDrop}
        onDragOver={onDragOver}
        onDragLeave={onDragLeave}
        className={`flex flex-col gap-2 rounded-2xl border bg-koma-panel px-3 py-2.5 shadow-sm transition-colors ${
          dragOver ? 'border-koma-accent bg-koma-hover' : 'border-koma-border'
        }`}
      >
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
          onChange={(e) => setInput(e.target.value)}
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
          </div>

          <div className="flex items-center gap-2">
            {working ? (
              // While the turn is running the send button MORPHS into a STOP
              // button — koma's Esc-interrupt equivalent (GuiReq Interrupt),
              // aborting the in-flight turn on the daemon.
              <button
                onClick={() => req({ r: 'Interrupt' })}
                aria-label="Stop"
                title="Stop"
                className="flex h-8 w-8 flex-none items-center justify-center rounded-full bg-koma-accent text-koma-bg transition-colors hover:opacity-90"
              >
                <Square size={14} className="fill-current" />
              </button>
            ) : (
              <button
                onClick={submit}
                disabled={!canSend}
                aria-label="Send"
                title="Send"
                className={`flex h-8 w-8 flex-none items-center justify-center rounded-full transition-colors ${
                  canSend
                    ? 'bg-koma-accent text-koma-bg hover:opacity-90'
                    : 'bg-koma-hover text-koma-fg opacity-40'
                }`}
              >
                <ArrowUp size={16} />
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}
