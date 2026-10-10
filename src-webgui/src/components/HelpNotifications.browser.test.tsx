import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'
import { render } from 'vitest-browser-react'
import HelpTab from './HelpTab'
import NotificationsTab from './NotificationsTab'
import { useKoma } from '../store/koma'
import { useNotifications, receiveNotifications } from '../lib/notifications'
import { startTour, stopTour, useGuide } from '../lib/tutorialTours'

const req = vi.fn()
const pause = (ms = 150) => new Promise(resolve => setTimeout(resolve, ms))
async function button(text: string) {
  const target = [...document.querySelectorAll('button')].find(b => b.textContent?.trim() === text)
  if (!target) throw new Error(`Missing button ${text}`)
  await userEvent.click(target)
}
beforeEach(() => {
  req.mockClear(); stopTour()
  useKoma.setState(s => ({ req, session: { ...s.session, id: 'browser-session' }, tutorial: { messages: [{ id: 'kept', role: 'assistant', content: 'Transcript retained' }], busy: false, error: null, pendingId: null, pendingTour: null } }))
  useNotifications.setState({ scopes: {}, errors: {} })
})
afterEach(() => { stopTour(); document.querySelectorAll('[data-test-guide]').forEach(el => el.remove()) })

describe('Help and notification browser flows', () => {
  it('searches offline reference and preserves Assistant transcript across sections', async () => {
    await render(<HelpTab />)
    await button('Reference')
    const search = document.querySelector<HTMLInputElement>('[aria-label="Search help"]')!
    await userEvent.type(search, 'notification')
    await button('Saved notification history')
    expect(document.body.textContent).toContain('Popup dismissal and expiry do not delete history')
    await button('Guides')
    expect(document.body.textContent).toContain('Terminal selection')
    await button('Assistant')
    expect(document.body.textContent).toContain('Transcript retained')
    expect(req).not.toHaveBeenCalled()
  })
  it('marks selected rows read and confirms scope-specific clearing', async () => {
    receiveNotifications({ id: 'list', session: 'browser-session', error: null, entries: [{ id: 'notice', timestamp: 1, severity: 'error', source: 'test', message: 'Browser notification', read: false }] })
    await render(<NotificationsTab />)
    const row = [...document.querySelectorAll('button')].find(b => b.textContent?.includes('Browser notification'))!
    await userEvent.click(row)
    expect(req.mock.calls.some(([r]) => r.request?.operation.op === 'read' && r.request.operation.id === 'notice')).toBe(true)
    await button('Clear history')
    expect(req.mock.calls.some(([r]) => r.request?.operation.op === 'clear')).toBe(false)
    await button('Confirm clear')
    expect(req.mock.calls.find(([r]) => r.request?.operation.op === 'clear')?.[0].request.session).toBe('browser-session')
  })
  it('keeps Help open during a guide and cancels on session switching', async () => {
    await render(<NotificationsTab />)
    useKoma.getState().openHelpTab()
    expect(startTour('notification-history')).toBe(true)
    await pause()
    expect(useKoma.getState().ui.tabs.some(t => t.id === 'help')).toBe(true)
    expect(document.querySelector('.driver-active-element')?.getAttribute('data-tour')).toBe('notification-history')
    useKoma.setState(s => ({ session: { ...s.session, id: 'other-session' } }))
    await pause()
    expect(useGuide.getState().id).toBeNull()
    expect(useGuide.getState().blocked).toContain('session changed')
    expect(document.querySelector('.driver-overlay')).toBeNull()
  })
  it('pauses hidden targets and cancellation stops pending navigation', async () => {
    const hidden = document.createElement('div')
    hidden.dataset.testGuide = 'true'; hidden.dataset.tour = 'activity-bar'; hidden.style.display = 'none'
    document.body.append(hidden)
    startTour('activity-bar')
    await pause(250)
    expect(useGuide.getState().blocked).toContain('unavailable')
    expect(hidden.classList.contains('driver-active-element')).toBe(false)
    stopTour()
    await pause()
    expect(document.querySelector('.driver-overlay')).toBeNull()
  })
  it('observes file opening and a successful save without performing the save', async () => {
    await render(<div><button data-tour-view="coding">Coding</button><div data-tour="coding-panel">Workspace tree</div><div data-tour="code-editor">File editor</div></div>)
    useKoma.setState(s => ({ coding: { ...s.coding, activeRoot: '/project', files: {} } }))
    startTour('coding-file-saves')
    await pause(200)
    useKoma.setState(s => ({ ui: { ...s.ui, tabs: [...s.ui.tabs, { id: 'test-file', kind: 'codingFile', root: '/project', path: 'a.ts', title: 'a.ts' }], activeTabId: 'test-file' } }))
    await pause(400)
    expect(useGuide.getState().step).toBe(1)
    useKoma.setState(s => ({ coding: { ...s.coding, files: { file: { dirty: true, fingerprint: 'before', saving: false, error: null, conflict: false } as any } } }))
    useKoma.setState(s => ({ coding: { ...s.coding, files: { file: { ...s.coding.files.file, dirty: false } } } }))
    await pause(300)
    expect(useGuide.getState().id).toBe('coding-file-saves') // discard is not a save
    useKoma.setState(s => ({ coding: { ...s.coding, files: { file: { ...s.coding.files.file, fingerprint: 'after-save' } } } }))
    await pause(400)
    expect(useGuide.getState().id).toBeNull()
    expect(req.mock.calls.some(([r]) => r.r === 'FileSave')).toBe(false)
  })
  it('requires a user-opened terminal and leaves unsaved file state intact', async () => {
    useKoma.setState(s => ({ ui: { ...s.ui, tabs: s.ui.tabs.filter(t => t.kind !== 'terminal') }, coding: { ...s.coding, files: { unsaved: { dirty: true, saving: false } as any } } }))
    startTour('terminal-selection')
    await pause()
    expect(useGuide.getState().blocked).toContain('Open a terminal tab yourself')
    expect(useKoma.getState().coding.files.unsaved.dirty).toBe(true)
    expect(req.mock.calls.some(([r]) => r.r === 'FileSave' || r.r === 'TerminalCreate')).toBe(false)
  })
})
