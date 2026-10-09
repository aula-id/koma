import { useState } from 'react'
import TutorialTab from './TutorialTab'
import LegacyHelpReference from './LegacyHelpReference'
import { MessageBody } from './MessageBody'
import { HELP_MANIFEST, helpArticle, rankHelp, helpRegistry } from '../lib/helpKnowledge'
import { TOUR_CATALOGUE, startTour, stopTour, useGuide, openHelpView } from '../lib/tutorialTours'

export default function HelpTab() {
  const [section, setSection] = useState<'Assistant' | 'Reference' | 'Guides'>('Assistant')
  const [query, setQuery] = useState('')
  const [topic, setTopic] = useState('help')
  const guide = useGuide()
  return <div className="flex h-full min-w-0 flex-col bg-koma-bg text-koma-fg">
    <nav aria-label="Help sections" className="flex gap-4 border-b border-koma-border p-3">{(['Assistant', 'Reference', 'Guides'] as const).map(s => <button key={s} aria-pressed={section === s} onClick={() => setSection(s)}>{s}</button>)}</nav>
    {(guide.id || guide.blocked) && <div role="status" className="border-b border-koma-border p-3 text-sm">Guide: {guide.id} · Step {guide.step + 1} {guide.blocked && <span>· Paused: {guide.blocked}</span>} <button onClick={stopTour}>Cancel guide</button></div>}
    {/* Keep the coach mounted so input and transcript survive section changes. */}
    <div className={section === 'Assistant' ? 'min-h-0 flex-1' : 'hidden'}><TutorialTab onArticle={id => { setTopic(id); setSection('Reference') }} /></div>
    {section === 'Reference' && <div className="flex min-h-0 flex-1">
      <nav className="w-56 shrink-0 overflow-auto border-r border-koma-border p-3"><input aria-label="Search help" value={query} onChange={e => setQuery(e.target.value)} placeholder="Search Reference" className="mb-3 w-full bg-koma-panel2 p-2" />{rankHelp(query).map(a => <button className="block py-2 text-left text-sm" key={a.id} onClick={() => setTopic(a.id)}>{a.title}</button>)}</nav>
      <article className="min-w-0 flex-1 overflow-auto p-5"><MessageBody text={helpArticle(topic)} />
        <button className="my-4" onClick={() => openHelpView(HELP_MANIFEST.articles.find(a => a.id === topic)?.navigation ?? 'help')}>Open view</button>
        <details><summary>TUI command and keybinding registry</summary>{(['COMMANDS', 'KEYBINDINGS'] as const).map(name => <div key={name}>{helpRegistry(name).map(r => <p key={r.key}><code>{r.key}</code> — {r.description}</p>)}</div>)}</details>
        <details className="mt-5"><summary>GUI controls and keyboard reference</summary><div className="h-[600px]"><LegacyHelpReference /></div></details>
      </article>
    </div>}
    {section === 'Guides' && <div className="flex-1 overflow-auto p-5"><p className="mb-4 text-sm opacity-60">Guides work offline. You perform all saves, connections, installs and deletions.</p>{TOUR_CATALOGUE.map(t => <div className="border-b border-koma-border py-3" key={t.id}><strong>{t.title}</strong><p className="text-sm opacity-60">{t.blurb}</p><button className="mt-2" onClick={() => startTour(t.id)}>Start guide</button></div>)}</div>}
  </div>
}
