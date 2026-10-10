import { useRef, useState } from 'react'
import { BookOpen, GraduationCap, Library } from 'lucide-react'
import TutorialTab from './TutorialTab'
import LegacyHelpReference from './LegacyHelpReference'
import { MessageBody } from './MessageBody'
import { HELP_MANIFEST, helpArticle, rankHelp, helpRegistry } from '../lib/helpKnowledge'
import { TOUR_CATALOGUE, startTour, stopTour, useGuide, openHelpView } from '../lib/tutorialTours'

const HELP_SECTIONS = [
  { id: 'Assistant', label: 'Assistant', Icon: GraduationCap },
  { id: 'Reference', label: 'Reference', Icon: BookOpen },
  { id: 'Guides', label: 'Guides', Icon: Library },
] as const

type HelpSection = (typeof HELP_SECTIONS)[number]['id']

export default function HelpTab() {
  const [section, setSection] = useState<HelpSection>('Assistant')
  const [query, setQuery] = useState('')
  const [topic, setTopic] = useState('help')
  const guide = useGuide()
  const tabRefs = useRef<Partial<Record<HelpSection, HTMLButtonElement | null>>>({})

  const selectSection = (next: HelpSection) => setSection(next)

  return (
    <div className="flex h-full min-w-0 flex-col bg-koma-bg text-koma-fg">
      {/* Sub-tab strip — mirrors main TabBar (top accent + panel bg when active). */}
      <nav
        role="tablist"
        aria-label="Help sections"
        className="flex h-9 flex-none items-stretch border-b border-koma-border bg-koma-panel2"
      >
        {HELP_SECTIONS.map((s, index) => {
          const active = section === s.id
          const { Icon } = s
          return (
            <button
              key={s.id}
              ref={(el) => {
                tabRefs.current[s.id] = el
              }}
              type="button"
              role="tab"
              id={`help-section-${s.id}`}
              aria-selected={active}
              aria-controls={`help-panel-${s.id}`}
              tabIndex={active ? 0 : -1}
              onClick={() => selectSection(s.id)}
              onKeyDown={(event) => {
                const next =
                  event.key === 'ArrowRight'
                    ? (index + 1) % HELP_SECTIONS.length
                    : event.key === 'ArrowLeft'
                      ? (index + HELP_SECTIONS.length - 1) % HELP_SECTIONS.length
                      : event.key === 'Home'
                        ? 0
                        : event.key === 'End'
                          ? HELP_SECTIONS.length - 1
                          : null
                if (next === null) return
                event.preventDefault()
                const id = HELP_SECTIONS[next].id
                selectSection(id)
                tabRefs.current[id]?.focus()
              }}
              className={`group relative flex h-full flex-none cursor-pointer select-none items-center gap-1.5 border-r border-koma-border px-3 text-[12px] transition-colors outline-none focus-visible:bg-koma-hover ${
                active
                  ? 'bg-koma-bg text-koma-fg'
                  : 'text-koma-dim hover:bg-koma-hover hover:text-koma-fg'
              }`}
            >
              {active && <span aria-hidden className="absolute inset-x-0 top-0 h-0.5 bg-koma-fg" />}
              <Icon size={13} className="opacity-80" aria-hidden />
              <span>{s.label}</span>
            </button>
          )
        })}
      </nav>

      {(guide.id || guide.blocked) && (
        <div role="status" className="border-b border-koma-border p-3 text-sm">
          Guide: {guide.id} · Step {guide.step + 1}{' '}
          {guide.blocked && <span>· Paused: {guide.blocked}</span>}{' '}
          <button type="button" onClick={stopTour}>
            Cancel guide
          </button>
        </div>
      )}

      {/* Keep the coach mounted so input and transcript survive section changes. */}
      <div
        role="tabpanel"
        id="help-panel-Assistant"
        aria-labelledby="help-section-Assistant"
        hidden={section !== 'Assistant'}
        className={section === 'Assistant' ? 'min-h-0 flex-1' : 'hidden'}
      >
        <TutorialTab
          onArticle={(id) => {
            setTopic(id)
            setSection('Reference')
          }}
        />
      </div>

      {section === 'Reference' && (
        <div
          role="tabpanel"
          id="help-panel-Reference"
          aria-labelledby="help-section-Reference"
          className="flex min-h-0 flex-1"
        >
          <nav className="w-56 shrink-0 overflow-auto border-r border-koma-border p-3">
            <input
              aria-label="Search help"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search Reference"
              className="mb-3 w-full bg-koma-panel2 p-2"
            />
            {rankHelp(query).map((a) => (
              <button className="block py-2 text-left text-sm" key={a.id} type="button" onClick={() => setTopic(a.id)}>
                {a.title}
              </button>
            ))}
          </nav>
          <article className="min-w-0 flex-1 overflow-auto p-5">
            <MessageBody text={helpArticle(topic)} />
            <button
              type="button"
              className="my-4"
              onClick={() =>
                openHelpView(HELP_MANIFEST.articles.find((a) => a.id === topic)?.navigation ?? 'help')
              }
            >
              Open view
            </button>
            <details>
              <summary>TUI command and keybinding registry</summary>
              {(['COMMANDS', 'KEYBINDINGS'] as const).map((name) => (
                <div key={name}>
                  {helpRegistry(name).map((r) => (
                    <p key={r.key}>
                      <code>{r.key}</code> — {r.description}
                    </p>
                  ))}
                </div>
              ))}
            </details>
            <details className="mt-5">
              <summary>GUI controls and keyboard reference</summary>
              <div className="h-[600px]">
                <LegacyHelpReference />
              </div>
            </details>
          </article>
        </div>
      )}

      {section === 'Guides' && (
        <div
          role="tabpanel"
          id="help-panel-Guides"
          aria-labelledby="help-section-Guides"
          className="flex-1 overflow-auto p-5"
        >
          <p className="mb-4 text-sm opacity-60">
            Guides work offline. You perform all saves, connections, installs and deletions.
          </p>
          {TOUR_CATALOGUE.map((t) => (
            <div className="border-b border-koma-border py-3" key={t.id}>
              <strong>{t.title}</strong>
              <p className="text-sm opacity-60">{t.blurb}</p>
              <button type="button" className="mt-2" onClick={() => startTour(t.id)}>
                Start guide
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
