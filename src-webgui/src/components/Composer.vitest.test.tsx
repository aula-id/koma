import { act, cleanup, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useKoma } from '../store/koma'

vi.mock('./lexical/LexicalMarkdownEditor', () => ({
  LexicalMarkdownEditor: ({ markdown }: { markdown: string }) => <textarea aria-label="chat draft" value={markdown} readOnly />,
}))
vi.mock('./CatMascot', () => ({ CatMascot: () => null }))
vi.mock('./ModelPicker', () => ({ ModelPicker: () => null }))
vi.mock('./EffortPicker', () => ({ EffortPicker: () => null }))
vi.mock('./ModeSelector', () => ({ ModeSelector: () => null }))

import { Composer } from './Composer'

beforeEach(() => {
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1 })
  window.ipc = { postMessage: vi.fn() }
  useKoma.setState((state) => ({
    session: { ...state.session, id: null, attachments: [], pendingSteer: [] },
    ui: {
      ...state.ui,
      tabs: [{ id: 'chat', kind: 'chat' }],
      activeTabId: 'chat',
      groups: ['g0'], tabGroup: { chat: 'g0' }, groupActive: { g0: 'chat' }, activeGroupId: 'g0',
      preserveTabsOnNextSession: false, preservedTabLayout: null, preservedTabsTargetSession: null,
      composerRefill: null,
    },
  }))
})
afterEach(() => { cleanup(); vi.unstubAllGlobals() })

it('keeps a guided Skills template until the target chat attaches, then displays it', () => {
  render(<Composer />)
  act(() => {
    useKoma.getState().newSessionPreservingTabs()
    useKoma.getState().refillComposer('Help me edit the Koma skill "alpha".')
    useKoma.getState().push({ k: 'Switching', to: 'guided-chat' })
  })
  expect((screen.getByRole('textbox', { name: 'chat draft' }) as HTMLTextAreaElement).value).toBe('')
  expect(useKoma.getState().ui.composerRefill).toContain('Help me edit')

  act(() => {
    useKoma.getState().push({
      k: 'Snapshot', session: 'guided-chat', state: 'idle', messages: [], title: '',
      subagents: [], bash: [], attachments: [], palette: useKoma.getState().palette,
    } as any)
  })
  expect((screen.getByRole('textbox', { name: 'chat draft' }) as HTMLTextAreaElement).value).toBe('Help me edit the Koma skill "alpha".')
  expect(useKoma.getState().ui.composerRefill).toBeNull()
})
