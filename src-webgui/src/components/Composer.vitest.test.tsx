import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
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

it('stages image-only selections from an empty draft, renders the snapshot inventory, and submits their markers', async () => {
  render(<Composer />)
  expect((screen.getByRole('textbox', { name: 'chat draft' }) as HTMLTextAreaElement).value).toBe('')
  const picker = document.querySelector('input[type="file"]') as HTMLInputElement
  const images = [
    new File(['first'], 'first.png', { type: 'image/png' }),
    new File(['second'], 'second.jpg', { type: 'image/jpeg' }),
  ]

  fireEvent.change(picker, { target: { files: images } })
  await waitFor(() => {
    const requests = (window.ipc!.postMessage as ReturnType<typeof vi.fn>).mock.calls.map(([wire]) => JSON.parse(wire))
    expect(requests.filter((request) => request.r === 'AttachFile')).toHaveLength(2)
  })

  act(() => {
    useKoma.getState().push({
      k: 'Snapshot', session: 'attachment-only', state: 'idle', messages: [], title: '',
      subagents: [], bash: [], pendingSteer: [], palette: useKoma.getState().palette,
      attachments: [
        { markerN: 1, name: 'first.png', kind: 'image' },
        { markerN: 2, name: 'second.jpg', kind: 'image' },
      ],
    } as any)
  })

  expect(screen.getAllByRole('group')).toHaveLength(2)
  expect(screen.getByRole('group', { name: 'first.png' })).not.toBeNull()
  expect(screen.getByRole('group', { name: 'second.jpg' })).not.toBeNull()
  const send = screen.getByRole('button', { name: 'Send (Ctrl+Enter)' }) as HTMLButtonElement
  expect(send.disabled).toBe(false)
  fireEvent.click(send)

  const requests = (window.ipc!.postMessage as ReturnType<typeof vi.fn>).mock.calls.map(([wire]) => JSON.parse(wire))
  expect(requests.at(-1)).toMatchObject({ r: 'Submit', text: '[Image #1] [Image #2]' })
})
