import { resolveCodingReply } from './coding-service'
import activitySource from '../components/ActivityBar.tsx?raw'
import tabSource from '../store/types/tabs.ts?raw'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { HELP_MANIFEST, helpArticle, helpRegistry, parseHelpAnswer, rankHelp } from './helpKnowledge'
import questions from '../../../src-misc/help/questions.json'
import { TOUR_CATALOGUE, workflowSteps, startTour, stopTour, useGuide } from './tutorialTours'
import { useKoma } from '../store/koma'
import { helpContext } from './helpContext'
import { showToast } from './toast'
import { captureNotificationOrigin } from './notificationOrigins'
import { useNotifications, receiveNotifications } from './notifications'

const requests = vi.fn()
beforeEach(() => {
  stopTour(); requests.mockClear()
  useKoma.setState(s => ({ req: requests, session: { ...s.session, id: 'session-a' }, ui: { ...s.ui, toast: null }, tutorial: { messages: [], pendingId: null, busy: false, error: null, pendingTour: null } }))
  useNotifications.setState({ scopes: {}, errors: {} })
})
describe('Bundled grounded Help', () => {
  it('covers actual command and workflow registries', () => {
    for (const command of helpRegistry('COMMANDS')) expect(HELP_MANIFEST.articles.some(a => a.aliases.includes(command.key))).toBe(true)
    for (const [, view] of activitySource.split('export const ACTIVITY_BAR_ITEMS:')[1].split(']')[0].matchAll(/view: '([^']+)'/g)) expect(HELP_MANIFEST.navigation).toContain(view)
    for (const [, view] of tabSource.replace(/\/\/[^\n]*/g, '').matchAll(/kind: '([^']+)'/g)) expect(HELP_MANIFEST.views).toHaveProperty(view)
    expect(helpRegistry('KEYBINDINGS').length).toBeGreaterThan(5)
    expect(TOUR_CATALOGUE.map(t => t.id).sort()).toEqual([...HELP_MANIFEST.workflows].sort())
    for (const tour of TOUR_CATALOGUE) expect(workflowSteps(tour.id).length).toBeGreaterThan(0)
    for (const article of HELP_MANIFEST.articles) { expect(helpArticle(article.id)).toContain('#'); expect(HELP_MANIFEST.navigation).toContain(article.navigation) }
  })
  for (const example of questions) it(`retrieves articles: ${example.question}`, () => {
    expect(rankHelp(example.question).slice(0, 4).some(a => example.topics.includes(a.id))).toBe(true)
  })
  it('rejects unknown article/action IDs and malformed coach output', () => {
    expect(() => parseHelpAnswer('TOUR: git')).toThrow()
    expect(() => parseHelpAnswer(JSON.stringify({ answer: 'Hello', articles: ['unknown'] }))).toThrow()
    expect(() => parseHelpAnswer(JSON.stringify({ answer: 'Hello', articles: ['help'], navigation: 'script()' }))).toThrow()
    // Fences / prose / extra keys are accepted when the Answer fields are valid.
    expect(parseHelpAnswer('```json\n{"answer":"Open Help → Reference.","articles":["help"],"extra":1}\n```').answer).toContain('Reference')
    expect(parseHelpAnswer('Sure.\n{"answer":"Use Guides.","articles":["help"],"navigation":"help"}\n').navigation).toBe('help')
  })
  it('retains transcript and routes legacy tutorial to singleton Help', () => {
    useKoma.getState().openTutorialTab(); useKoma.getState().openHelpTab()
    expect(useKoma.getState().ui.tabs.filter(t => t.kind === 'help')).toHaveLength(1)
    expect(useKoma.getState().ui.activeTabId).toBe('help')
    useKoma.getState().sendTutorialChat('Explain notifications')
    const id = useKoma.getState().tutorial.pendingId!
    useKoma.getState().push({ k: 'TutorialChatDone', id, text: JSON.stringify({ answer: 'Open Notifications.', articles: ['notifications'], navigation: 'notifications' }), tour: null, error: null })
    useKoma.getState().activateTab('chat'); useKoma.getState().openHelpTab()
    expect(useKoma.getState().tutorial.messages).toHaveLength(2)
    useKoma.getState().push({ k: 'TutorialChatDone', id, text: 'stale', tour: null, error: null })
    expect(useKoma.getState().tutorial.messages).toHaveLength(2)
  })
  it('keeps multi-turn assistant wire as Answer JSON', () => {
    useKoma.getState().sendTutorialChat('Explain notifications')
    const id = useKoma.getState().tutorial.pendingId!
    useKoma.getState().push({ k: 'TutorialChatDone', id, text: JSON.stringify({ answer: 'Open Notifications.', articles: ['notifications'], navigation: 'notifications' }), tour: null, error: null })
    requests.mockClear()
    useKoma.getState().sendTutorialChat('how about in GUI?')
    const wire = requests.mock.calls.find(([r]) => r.r === 'TutorialChat')?.[0].messages
    expect(wire).toEqual(expect.arrayContaining([
      expect.objectContaining({ role: 'user', content: 'Explain notifications' }),
      expect.objectContaining({
        role: 'assistant',
        content: JSON.stringify({ answer: 'Open Notifications.', articles: ['notifications'], navigation: 'notifications', guide: null }),
      }),
      expect.objectContaining({ role: 'user', content: 'how about in GUI?' }),
    ]))
  })
  it('redacts UI context by constructing an allowlist', () => {
    useKoma.setState(s => ({ session: { ...s.session, title: 'private-title', stream: 'secret output', messages: [{ role: 'user', content: 'private chat' } as any] }, config: { ...s.config, providers: [{ apiKey: 'secret-key' } as any] } }))
    const json = JSON.stringify(helpContext())
    for (const secret of ['private-title', 'secret output', 'private chat', 'secret-key']) expect(json).not.toContain(secret)
  })
  it('explains missing guide prerequisites without changing configuration', () => {
    useKoma.setState(s => ({ session: { ...s.session, id: null } }))
    expect(startTour('coding-file-saves')).toBe(false)
    expect(useGuide.getState().blocked).toContain('Attach a session')
    stopTour(); expect(useGuide.getState().id).toBeNull()
  })
})
describe('Notification capture and scope', () => {
  it('captures identical events separately, before popup dismissal', () => {
    showToast('same'); showToast('same')
    const entries = requests.mock.calls.map(([r]) => r).filter(r => r.r === 'Notifications')
    expect(entries).toHaveLength(2)
    expect(entries[0].request.operation.entry.id).not.toBe(entries[1].request.operation.entry.id)
    useKoma.getState().dismissToast(useKoma.getState().ui.toast!.id)
    expect(requests).toHaveBeenCalledTimes(2)
  })
  it('captures legacy direct store writes through the same publisher', () => {
    useKoma.setState(s => ({ ui: { ...s.ui, toastSeq: s.ui.toastSeq + 1, toast: { id: s.ui.toastSeq + 1, text: 'legacy', kind: 'error' } } }))
    expect(requests.mock.calls[0][0].request.operation.entry.message).toBe('legacy')
  })
  it('uses originating scope when a correlated operation finishes after session switching', () => {
    captureNotificationOrigin({ r: 'SetAgent', reqSeq: 700, scope: 'session' }, 'session-a')
    useKoma.setState(s => ({ session: { ...s.session, id: 'session-b' }, agentSaving: { seq: 700 } as any }))
    useKoma.getState().push({ k: 'AgentOp', reqSeq: 700, ok: false, error: 'save failed' } as any)
    expect(requests.mock.calls.find(([r]) => r.r === 'Notifications')?.[0].request.session).toBe('session-a')
  })
  it('routes a late remote event through its retained originating host', () => {
    const postMessage = vi.fn()
    const previousIpc = window.ipc
    window.ipc = { postMessage }
    useKoma.setState(s => ({ session: { ...s.session, id: 'remote-origin-session' }, remoteState: { ...s.remoteState, hostId: 'remote-origin-host' } }))
    showToast('Remote operation started')
    useKoma.setState(s => ({ session: { ...s.session, id: 'local-new-session' }, remoteState: { ...s.remoteState, hostId: null } }))
    showToast('Remote operation finished', 'info', { session: 'remote-origin-session' })
    expect(postMessage).toHaveBeenCalledTimes(2)
    for (const [json] of postMessage.mock.calls) {
      const wire = JSON.parse(json).request
      expect(wire.workspace).toEqual({ hostId: 'remote-origin-host', root: '' })
      expect(wire.request.session).toBe('remote-origin-session')
      resolveCodingReply({ k: 'CodingReply', id: wire.id, workspace: wire.workspace, result: { id: wire.request.id, session: wire.request.session, entries: [], error: null } })
    }
    window.ipc = previousIpc
  })
  it('keeps App events local and deduplicates transport retries', () => {
    const event = { k: 'KeyOp', ok: false, error: 'key failed', eventId: 'event-retry' } as any
    useKoma.getState().push(event); useKoma.getState().push(event)
    const records = requests.mock.calls.filter(([r]) => r.r === 'Notifications')
    expect(records).toHaveLength(1)
    expect(records[0][0].request.session).toBeNull()
  })
  it('never reopens a dismissed runtime popup for repeated status frames', () => {
    const status = { k: 'Status', session: 'session-a', working: false, toast: 'runtime', toastKind: 'info', toastEventId: 'runtime-once', toastSession: 'session-a' } as const
    useKoma.getState().push(status)
    useKoma.getState().dismissToast(useKoma.getState().ui.toast!.id)
    useKoma.getState().push({ ...status, tokensIn: 4 })
    expect(useKoma.getState().ui.toast).toBeNull()
    expect(requests).not.toHaveBeenCalled()
  })
  it('isolates read/clear synchronization by scope and retains feedback on storage errors', () => {
    receiveNotifications({ id: 'a', session: 'session-a', entries: [{ id: 'e', timestamp: 1, severity: 'info', source: 'test', message: 'saved', read: false }], error: null })
    receiveNotifications({ id: 'b', session: null, entries: [], error: null })
    expect(useNotifications.getState().scopes['session-a']).toHaveLength(1)
    receiveNotifications({ id: 'c', session: 'session-a', entries: [], error: 'disk failed' })
    expect(useNotifications.getState().scopes['session-a']).toHaveLength(1)
    showToast('still immediate'); expect(useKoma.getState().ui.toast?.text).toBe('still immediate')
  })
})
