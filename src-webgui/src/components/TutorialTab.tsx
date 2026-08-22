import { useEffect, useMemo, useRef, useState, type FormEvent, type KeyboardEvent, type ReactNode } from 'react'
import { GraduationCap, Play, Send, Sparkles } from 'lucide-react'
import { useKoma } from '../store/koma'
import { MessageBody } from './MessageBody'
import { TutorialPaperclip, type PaperclipMood } from './TutorialPaperclip'
import {
  TOUR_CATALOGUE,
  startTour,
  tourMeta,
  type TourId,
} from '../lib/tutorialTours'

// In-app Tutorial tab: NLP coach over host-proxied koma-free + driver.js tours.
// Theme-aware end-to-end (koma-* tokens only). No daemon session required.

export default function TutorialTab() {
  const messages = useKoma((s) => s.tutorial.messages)
  const busy = useKoma((s) => s.tutorial.busy)
  const error = useKoma((s) => s.tutorial.error)
  const pendingTour = useKoma((s) => s.tutorial.pendingTour)
  const sendTutorialChat = useKoma((s) => s.sendTutorialChat)
  const clearTutorialPendingTour = useKoma((s) => s.clearTutorialPendingTour)
  const clearTutorialError = useKoma((s) => s.clearTutorialError)

  const [draft, setDraft] = useState('')
  const [showTopics, setShowTopics] = useState(true)
  const scrollRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLTextAreaElement>(null)

  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    el.scrollTop = el.scrollHeight
  }, [messages, busy, pendingTour, error])

  const mood: PaperclipMood = useMemo(() => {
    if (busy) return 'think'
    if (pendingTour) return 'point'
    if (messages.length === 0) return 'wave'
    return 'idle'
  }, [busy, pendingTour, messages.length])

  const pendingMeta = tourMeta(pendingTour)

  const submit = (text: string) => {
    const t = text.trim()
    if (!t || busy) return
    setDraft('')
    setShowTopics(false)
    clearTutorialError()
    sendTutorialChat(t)
  }

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    submit(draft)
  }

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      submit(draft)
    }
  }

  const launchTour = (id: string) => {
    clearTutorialPendingTour()
    startTour(id as TourId)
  }

  const setupTours = TOUR_CATALOGUE.filter((t) => t.kind === 'setup')
  const spotlights = TOUR_CATALOGUE.filter((t) => t.kind === 'spotlight')

  return (
    <div className="flex h-full w-full min-w-0 flex-col bg-koma-bg text-koma-fg">
      {/* Header */}
      <header className="flex flex-none items-center gap-2 border-b border-koma-border bg-koma-panel2 px-4 py-2.5">
        <GraduationCap size={16} className="text-koma-accent opacity-90" />
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-semibold">Tutorial</div>
          <div className="text-[11px] text-koma-fg opacity-45">
            Ask how to do something, or pick a guided tour. Powered by koma free (no session).
          </div>
        </div>
        <button
          type="button"
          onClick={() => setShowTopics((v) => !v)}
          className="rounded-md border border-koma-border px-2 py-1 text-[11px] text-koma-fg opacity-70 transition hover:bg-koma-hover hover:opacity-100"
        >
          {showTopics ? 'Hide topics' : 'Topics'}
        </button>
      </header>

      <div className="flex min-h-0 flex-1">
        {/* Topic rail */}
        {showTopics && (
          <aside className="flex w-56 flex-none flex-col gap-3 overflow-y-auto border-r border-koma-border bg-koma-panel2 p-3">
            <RailSection title="Setup recipes">
              {setupTours.map((t) => (
                <TopicButton
                  key={t.id}
                  title={t.title}
                  blurb={t.blurb}
                  onAsk={() => submit(`How do I ${t.title.toLowerCase()}?`)}
                  onTour={() => launchTour(t.id)}
                />
              ))}
            </RailSection>
            <RailSection title="Feature spotlights">
              {spotlights.map((t) => (
                <TopicButton
                  key={t.id}
                  title={t.title}
                  blurb={t.blurb}
                  onAsk={() => submit(`What is ${t.title}?`)}
                  onTour={() => launchTour(t.id)}
                />
              ))}
            </RailSection>
          </aside>
        )}

        {/* Chat column */}
        <div className="flex min-w-0 flex-1 flex-col">
          <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
            {messages.length === 0 && !busy && (
              <div className="mx-auto flex max-w-xl flex-col items-start gap-3 pt-6">
                <div className="flex items-center gap-2 text-koma-accent">
                  <Sparkles size={16} />
                  <span className="text-[13px] font-semibold">Ask the coach</span>
                </div>
                <p className="text-[12.5px] leading-relaxed text-koma-fg opacity-60">
                  Try natural language — any language works. Examples:
                </p>
                <div className="flex flex-wrap gap-1.5">
                  {[
                    'gimana cara konek provider',
                    'how do I add an API key model',
                    'what is the agents panel',
                    'how do I connect SSH remote',
                  ].map((q) => (
                    <button
                      key={q}
                      type="button"
                      onClick={() => submit(q)}
                      className="rounded-full border border-koma-border bg-koma-panel px-2.5 py-1 text-[11.5px] text-koma-fg opacity-75 transition hover:border-koma-accent/40 hover:bg-koma-hover hover:opacity-100"
                    >
                      {q}
                    </button>
                  ))}
                </div>
              </div>
            )}

            <div className="mx-auto flex max-w-2xl flex-col gap-3">
              {messages.map((m) => (
                <div
                  key={m.id}
                  className={`flex ${m.role === 'user' ? 'justify-end' : 'justify-start'}`}
                >
                  <div
                    className={`max-w-[85%] rounded-lg px-3 py-2 text-[12.5px] leading-relaxed ${
                      m.role === 'user'
                        ? 'bg-koma-band text-koma-fg'
                        : 'border border-koma-border bg-koma-panel text-koma-fg'
                    }`}
                  >
                    {m.role === 'assistant' ? (
                      <MessageBody text={m.content} />
                    ) : (
                      <div className="whitespace-pre-wrap">{m.content}</div>
                    )}
                    {m.tour && (
                      <button
                        type="button"
                        onClick={() => launchTour(m.tour!)}
                        className="mt-2 inline-flex items-center gap-1 rounded-md border border-koma-accent/40 bg-koma-accent/10 px-2 py-1 text-[11px] font-medium text-koma-accent transition hover:bg-koma-accent/20"
                      >
                        <Play size={11} />
                        Start tour: {tourMeta(m.tour)?.title ?? m.tour}
                      </button>
                    )}
                  </div>
                </div>
              ))}

              {busy && (
                <div className="flex justify-start">
                  <div className="rounded-lg border border-koma-border bg-koma-panel px-3 py-2 text-[12px] text-koma-fg opacity-55">
                    Thinking…
                  </div>
                </div>
              )}

              {error && (
                <div className="rounded-md border border-koma-error/40 bg-koma-error/10 px-3 py-2 text-[12px] text-koma-error">
                  {error}
                  <div className="mt-1 text-[11px] opacity-80">
                    Use Topics on the left for offline guided tours.
                  </div>
                </div>
              )}

              {pendingTour && pendingMeta && !busy && (
                <div className="flex items-center gap-2 rounded-md border border-koma-accent/35 bg-koma-accent/10 px-3 py-2">
                  <span className="min-w-0 flex-1 text-[12px] text-koma-fg">
                    Suggested tour: <strong className="text-koma-accent">{pendingMeta.title}</strong>
                    <span className="opacity-55"> — {pendingMeta.blurb}</span>
                  </span>
                  <button
                    type="button"
                    onClick={() => launchTour(pendingTour)}
                    className="flex-none rounded-md bg-koma-accent px-2.5 py-1 text-[11px] font-semibold text-koma-bg transition hover:opacity-90"
                  >
                    Start
                  </button>
                  <button
                    type="button"
                    onClick={() => clearTutorialPendingTour()}
                    className="flex-none rounded-md px-2 py-1 text-[11px] text-koma-fg opacity-55 hover:bg-koma-hover hover:opacity-90"
                  >
                    Dismiss
                  </button>
                </div>
              )}
            </div>
          </div>

          {/* Input + paperclip */}
          <form
            onSubmit={onSubmit}
            className="flex flex-none items-end gap-2 border-t border-koma-border bg-koma-panel2 px-3 py-2.5"
          >
            <TutorialPaperclip
              mood={mood}
              onClick={() => setShowTopics((v) => !v)}
              title="Toggle topics"
            />
            <textarea
              ref={inputRef}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={onKeyDown}
              rows={1}
              placeholder="Ask how to do something in the GUI…"
              disabled={busy}
              className="max-h-28 min-h-[40px] min-w-0 flex-1 resize-y rounded-md border border-koma-border bg-koma-bg px-3 py-2 text-[12.5px] text-koma-fg outline-none placeholder:text-koma-fg placeholder:opacity-35 focus:border-koma-accent/50 disabled:opacity-50"
            />
            <button
              type="submit"
              disabled={busy || !draft.trim()}
              aria-label="Send"
              className="flex h-10 w-10 flex-none items-center justify-center rounded-md bg-koma-accent text-koma-bg transition hover:opacity-90 disabled:opacity-35"
            >
              <Send size={15} />
            </button>
          </form>
        </div>
      </div>
    </div>
  )
}

function RailSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div>
      <div className="mb-1 px-1 text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-40">
        {title}
      </div>
      <div className="flex flex-col gap-1">{children}</div>
    </div>
  )
}

function TopicButton({
  title,
  blurb,
  onAsk,
  onTour,
}: {
  title: string
  blurb: string
  onAsk: () => void
  onTour: () => void
}) {
  return (
    <div className="rounded-md border border-koma-border bg-koma-bg/40 p-2">
      <div className="text-[12px] font-medium text-koma-fg">{title}</div>
      <div className="mt-0.5 text-[10.5px] leading-snug text-koma-fg opacity-50">{blurb}</div>
      <div className="mt-1.5 flex gap-1">
        <button
          type="button"
          onClick={onAsk}
          className="rounded px-1.5 py-0.5 text-[10.5px] text-koma-fg opacity-70 transition hover:bg-koma-hover hover:opacity-100"
        >
          Ask
        </button>
        <button
          type="button"
          onClick={onTour}
          className="inline-flex items-center gap-0.5 rounded px-1.5 py-0.5 text-[10.5px] text-koma-accent transition hover:bg-koma-hover"
        >
          <Play size={10} /> Tour
        </button>
      </div>
    </div>
  )
}
