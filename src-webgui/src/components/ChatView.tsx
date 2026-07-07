import { useState, type KeyboardEvent } from 'react'
import { ChevronDown, ChevronRight, Loader2, Send } from 'lucide-react'
import { useKoma, type ChatMessage } from '../store/koma'

// Round-trips a message's reasoning (collapsed by default) above its content.
function Bubble({ role, content, reasoning }: ChatMessage) {
  const [reasoningOpen, setReasoningOpen] = useState(false)
  const isUser = role === 'user'

  return (
    <div className={`flex w-full ${isUser ? 'justify-end' : 'justify-start'}`}>
      <div
        className={`max-w-[80%] rounded-md border px-3 py-2 text-[13px] leading-relaxed whitespace-pre-wrap ${
          isUser
            ? 'border-koma-border bg-koma-panel text-koma-fg'
            : 'border-koma-border bg-koma-panel2 text-koma-fg'
        }`}
      >
        {reasoning && (
          <div className="mb-1.5 border-b border-koma-border pb-1.5">
            <button
              onClick={() => setReasoningOpen((o) => !o)}
              className="flex items-center gap-1 text-[11px] text-koma-fg opacity-50 transition-colors hover:opacity-80"
            >
              {reasoningOpen ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
              reasoning
            </button>
            {reasoningOpen && (
              <div className="mt-1 whitespace-pre-wrap text-[12px] text-koma-fg opacity-60">
                {reasoning}
              </div>
            )}
          </div>
        )}
        <div>{content}</div>
      </div>
    </div>
  )
}

// Native chat view — replaces the xterm Terminal. Reads the koma store
// (mirror of the host's authoritative push envelopes) and never accumulates
// state locally: history bubbles come straight from session.messages, and
// the in-flight assistant reply is a single live bubble driven by
// session.stream/session.reasoning. That live bubble is keyed at its FUTURE
// index (messages.length) so that when the Snapshot commit lands and the
// message joins the array, React reuses the same DOM node instead of
// remounting it.
export function ChatView() {
  const messages = useKoma((s) => s.session.messages)
  const stream = useKoma((s) => s.session.stream)
  const reasoning = useKoma((s) => s.session.reasoning)
  const working = useKoma((s) => s.session.working)
  const req = useKoma((s) => s.req)
  const [input, setInput] = useState('')

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

  return (
    <div className="term-shell flex flex-col">
      <div className="flex-1 space-y-3 overflow-y-auto px-2 py-4">
        {messages.map((m, i) => (
          <Bubble key={i} role={m.role} content={m.content} reasoning={m.reasoning} />
        ))}
        {stream.length > 0 && (
          <Bubble key={messages.length} role="assistant" content={stream} reasoning={reasoning || null} />
        )}
      </div>
      <div className="flex items-end gap-2 border-t border-koma-border px-2 py-2">
        {working && <Loader2 size={14} className="flex-none animate-spin text-koma-fg opacity-60" />}
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={onKeyDown}
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
