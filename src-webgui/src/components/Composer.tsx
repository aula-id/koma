import {
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type DragEvent,
  type KeyboardEvent,
} from 'react'
import { Loader2, Paperclip, Search, Send, X } from 'lucide-react'
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
  const [input, setInput] = useState('')
  const [dragOver, setDragOver] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)

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

  return (
    <div
      onDrop={onDrop}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      className={`flex flex-col gap-1.5 border-t border-koma-border px-2 py-2 transition-colors ${
        dragOver ? 'bg-koma-hover' : ''
      }`}
    >
      {attachments.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {attachments.map((a) => (
            <span
              key={a.markerN}
              className="flex items-center gap-1 rounded border border-koma-border bg-koma-panel px-1.5 py-0.5 text-[11px] text-koma-fg opacity-80"
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
      <div className="flex items-end gap-2">
        {working && <Loader2 size={14} className="flex-none animate-spin text-koma-fg opacity-60" />}
        <input ref={fileInputRef} type="file" multiple className="hidden" onChange={onFilePicked} />
        <button
          onClick={() => fileInputRef.current?.click()}
          aria-label="Attach file"
          title="Attach file"
          className="flex h-[32px] w-[32px] flex-none items-center justify-center rounded-md border border-koma-border bg-koma-panel text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          <Paperclip size={14} />
        </button>
        <button
          onClick={openOmniSearch}
          aria-label="Search workspace files"
          title="Search workspace files"
          className="flex h-[32px] w-[32px] flex-none items-center justify-center rounded-md border border-koma-border bg-koma-panel text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          <Search size={14} />
        </button>
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
          placeholder="Message koma…"
          rows={1}
          className="min-h-[32px] flex-1 resize-none rounded-md border border-koma-border bg-koma-panel px-2.5 py-1.5 text-[13px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-40"
        />
        <button
          onClick={submit}
          disabled={!input.trim()}
          aria-label="Send"
          className="flex h-[32px] w-[32px] flex-none items-center justify-center rounded-md border border-koma-border bg-koma-panel text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100 disabled:opacity-30"
        >
          <Send size={14} />
        </button>
      </div>
    </div>
  )
}
