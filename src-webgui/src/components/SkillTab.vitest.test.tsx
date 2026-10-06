import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { useKoma, type SkillCatalogueEntry, type SkillDetail, type Tab } from '../store/koma'

vi.mock('./MessageBody', () => ({ MessageBody: ({ text }: { text: string }) => <div data-testid="markdown-preview">{text}</div> }))

import SkillTab from './SkillTab'

const entry: SkillCatalogueEntry = {
  skillId: 'opaque-a', generation: 'gen-a', name: 'alpha', description: 'Draft description', triggers: '',
  sourceTier: 'global', scope: 'global', sourcePath: '/skills/alpha/SKILL.md',
}
const detail: SkillDetail = {
  skillId: 'opaque-a', generation: 'gen-a', name: 'alpha', description: 'Draft description', triggers: '',
  declaredTools: ['Read'], instruction: 'Draft body', companionFiles: [], scope: 'global',
  sourcePath: '/skills/alpha/SKILL.md', editable: true, structuredSaveSupported: true,
}
const tab = { id: 'skill:opaque-a', kind: 'skill' as const, skillId: 'opaque-a', title: 'alpha' }
const otherSkillTab = { id: 'skill:opaque-b', kind: 'skill' as const, skillId: 'opaque-b', title: 'beta' }

beforeEach(() => {
  useKoma.setState((state) => ({
    session: { ...state.session, id: null },
    ui: {
      ...state.ui,
      tabs: [{ id: 'chat', kind: 'chat' }],
      activeTabId: 'chat',
      groups: ['g0'],
      tabGroup: { chat: 'g0' },
      groupActive: { g0: 'chat' },
      activeGroupId: 'g0',
      groupSizes: { g0: 1 },
      composerRefill: null,
      preserveTabsOnNextSession: false,
    },
    skills: [], loadedSkillNames: [], skillDetails: { [tab.id]: detail }, skillDetailPending: {},
    skillDetailErrors: {}, skillFiles: {}, skillLastOp: null, skillOpResults: {}, skillSessionEpoch: 4,
  }))
})
afterEach(() => {
  cleanup()
  vi.useRealTimers()
  delete window.ipc
})

describe('SkillTab parity and stale safety', () => {
  it('rehydrates a retained Skill tab after discovery without a manual Refresh', () => {
    const postMessage = vi.fn()
    window.ipc = { postMessage }
    useKoma.setState((state) => ({
      session: { ...state.session, id: 'new-chat' },
      skillsLoading: true,
      skillRequestId: 'post-attach',
      skillDetails: {},
      ui: { ...state.ui, tabs: [{ id: 'chat', kind: 'chat' }, tab] },
    }))
    render(<SkillTab tab={tab} />)
    expect(screen.queryByText('Skill no longer available.')).toBeNull()

    act(() => useKoma.getState().push({ k: 'SkillValues', requestId: 'post-attach', sessionEpoch: 4, skills: [entry], loadedSkillNames: [], error: null }))
    const request = postMessage.mock.calls.map(([message]) => JSON.parse(message)).find((sent) => sent.r === 'GetSkillDetail')
    expect(request).toMatchObject({ tabId: tab.id, skillId: entry.skillId, generation: entry.generation, sessionEpoch: 4 })
    act(() => useKoma.getState().push({ k: 'SkillDetailValues', requestId: request.requestId, sessionEpoch: 4, tabId: tab.id, detail, filePath: null, fileContent: null, error: null }))
    expect(screen.getByText('SKILL.md')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Edit with Koma' })).toBeTruthy()
    expect(screen.queryByText('Skill no longer available.')).toBeNull()
  })

  it('preserves a disappeared-source draft read-only with safe actions', () => {
    render(<SkillTab tab={tab} />)
    expect(screen.getByText(/no longer the current catalogue winner/)).toBeTruthy()
    expect(screen.getByRole('textbox', { name: 'Description' })).toHaveProperty('readOnly', true)
    expect(screen.getByRole('button', { name: 'Edit with Koma' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Download .zip' })).toHaveProperty('disabled', true)
    expect(screen.getByRole('button', { name: 'Delete' })).toHaveProperty('disabled', true)
    expect(screen.getByRole('button', { name: 'Load' })).toHaveProperty('disabled', true)
    expect(screen.queryByRole('button', { name: 'Duplicate to Koma' })).toBeNull()
  })

  it('directs detached External Edit with Koma into an owned duplicate workflow', () => {
    const postMessage = vi.fn()
    window.ipc = { postMessage }
    useKoma.setState({
      skills: [{ ...entry, scope: 'external', sourceTier: 'extra-root' }],
      skillDetails: { [tab.id]: { ...detail, scope: 'external', editable: false, structuredSaveSupported: false } },
    })
    render(<SkillTab tab={tab} />)
    fireEvent.click(screen.getByRole('button', { name: 'Edit with Koma' }))
    const requests = postMessage.mock.calls.map(([message]) => JSON.parse(message))
    expect(requests.filter((request) => request.r === 'NewSession')).toHaveLength(1)
    expect(useKoma.getState().ui.preserveTabsOnNextSession).toBe(true)
    expect(useKoma.getState().ui.composerRefill).toContain('Do not modify it')
    expect(useKoma.getState().ui.composerRefill).toContain('Duplicate to Koma')
    expect(screen.queryByRole('button', { name: 'Download .zip' })).toBeNull()
    expect(screen.queryByText(/uses advanced YAML/)).toBeNull()
  })

  it('reuses an active chat for Edit with Koma and preserves every editor tab', () => {
    const postMessage = vi.fn()
    window.ipc = { postMessage }
    const tabs: Tab[] = [
      { id: 'chat', kind: 'chat' },
      tab,
      otherSkillTab,
      { id: 'upload-skill', kind: 'uploadSkill' },
    ]
    useKoma.setState((state) => ({
      session: { ...state.session, id: 'session-a' },
      skills: [entry],
      ui: {
        ...state.ui,
        tabs,
        activeTabId: tab.id,
        tabGroup: { chat: 'g0', [tab.id]: 'g0', [otherSkillTab.id]: 'g0', 'upload-skill': 'g0' },
        groupActive: { g0: tab.id },
      },
    }))

    render(<SkillTab tab={tab} />)
    fireEvent.click(screen.getByRole('button', { name: 'Edit with Koma' }))

    const requests = postMessage.mock.calls.map(([message]) => JSON.parse(message))
    expect(requests.some((request) => request.r === 'NewSession')).toBe(false)
    expect(useKoma.getState().ui.preserveTabsOnNextSession).toBe(false)
    expect(useKoma.getState().ui.tabs.map((openTab) => openTab.id)).toEqual(['chat', tab.id, otherSkillTab.id, 'upload-skill'])
    expect(useKoma.getState().ui.activeTabId).toBe('chat')
    expect(useKoma.getState().ui.composerRefill).toContain('Help me edit the Koma skill "alpha".')
    expect(useKoma.getState().ui.composerRefill).toContain('reopen this skill and save the agreed version')
  })

  it('keeps the Save acknowledgment even when an unrelated SkillOp arrives later', () => {
    window.ipc = { postMessage: vi.fn() }
    useKoma.setState((state) => ({ session: { ...state.session, id: 'session-a' }, skills: [entry] }))
    render(<SkillTab tab={tab} />)
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    const calls = vi.mocked(window.ipc!.postMessage).mock.calls
    const request = JSON.parse(calls[calls.length - 1][0])
    expect(request.r).toBe('UpdateSkill')
    act(() => useKoma.getState().push({ k: 'SkillOp', requestId: request.requestId, sessionEpoch: 4, tabId: tab.id, operation: 'update', outcomes: [{ name: 'alpha', status: 'success' }], loadedSkillNames: [] }))
    act(() => useKoma.getState().push({ k: 'SkillOp', requestId: 'another-tab', sessionEpoch: 4, tabId: 'skills-panel', operation: 'load', outcomes: [{ name: 'beta', status: 'success' }], loadedSkillNames: ['beta'] }))
    expect(useKoma.getState().ui.toast?.text).toContain('update completed')
    expect(useKoma.getState().ui.toast?.kind).toBe('success')
    expect(screen.getByRole('button', { name: 'Save' }).querySelector('[aria-hidden]')).toBeNull()
  })

  it('stops an unconfirmed Save spinner without claiming that disk was unchanged', () => {
    vi.useFakeTimers()
    window.ipc = { postMessage: vi.fn() }
    useKoma.setState((state) => ({ session: { ...state.session, id: 'session-a' }, skills: [entry] }))
    render(<SkillTab tab={tab} />)
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(screen.getByRole('button', { name: 'Save' }).querySelector('[aria-hidden]')).toBeTruthy()
    act(() => vi.advanceTimersByTime(15_000))
    expect(useKoma.getState().ui.toast?.text).toContain('did not confirm')
    expect(useKoma.getState().ui.toast?.kind).toBe('warn')
    expect(screen.getByRole('button', { name: 'Save' }).querySelector('[aria-hidden]')).toBeNull()
    expect(screen.getByRole('button', { name: 'Save' })).toHaveProperty('disabled', true)
  })

  it('marks an unacknowledged Save as unknown immediately after a session switch', () => {
    window.ipc = { postMessage: vi.fn() }
    useKoma.setState((state) => ({ session: { ...state.session, id: 'session-a' }, skills: [entry] }))
    render(<SkillTab tab={tab} />)
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    act(() => useKoma.setState({ skillSessionEpoch: 5, skillOpResults: {} }))
    expect(useKoma.getState().ui.toast?.text).toContain('did not confirm')
    expect(useKoma.getState().ui.toast?.kind).toBe('warn')
    expect(screen.getByRole('button', { name: 'Save' }).querySelector('[aria-hidden]')).toBeNull()
  })

  it('offers one correlated Save & Reload request only to an owned skill loaded in this chat', () => {
    const postMessage = vi.fn()
    window.ipc = { postMessage }
    useKoma.setState((state) => ({ session: { ...state.session, id: 'session-a' }, skills: [entry], loadedSkillNames: ['alpha'] }))
    render(<SkillTab tab={tab} />)
    expect(screen.getByRole('button', { name: 'Reload' }).title).toBe('Reload from disk')
    fireEvent.click(screen.getByRole('button', { name: 'Save & Reload' }))
    const updates = postMessage.mock.calls.map(([message]) => JSON.parse(message)).filter((request) => request.r === 'UpdateSkill')
    expect(updates).toHaveLength(1)
    expect(updates[0]).toMatchObject({ reloadAfterSave: true, targetSessionId: 'session-a', sessionEpoch: 4, tabId: tab.id })
    expect(postMessage.mock.calls.some(([message]) => JSON.parse(message).r === 'ReloadSkills')).toBe(false)
    act(() => useKoma.getState().push({ k: 'SkillOp', requestId: updates[0].requestId, sessionEpoch: 4, tabId: tab.id, operation: 'update-reload', outcomes: [{ name: 'alpha', status: 'success' }], loadedSkillNames: ['alpha'] }))
    expect(useKoma.getState().ui.toast?.text).toContain('Saved and reloaded in this chat.')
    expect(useKoma.getState().ui.toast?.kind).toBe('success')
  })

  it('reports disk-only partial success without claiming that the chat reloaded', () => {
    window.ipc = { postMessage: vi.fn() }
    useKoma.setState((state) => ({ session: { ...state.session, id: 'session-a' }, skills: [entry], loadedSkillNames: ['alpha'] }))
    render(<SkillTab tab={tab} />)
    fireEvent.click(screen.getByRole('button', { name: 'Save & Reload' }))
    const calls = vi.mocked(window.ipc!.postMessage).mock.calls
    const request = JSON.parse(calls[calls.length - 1][0])
    act(() => useKoma.getState().push({ k: 'SkillOp', requestId: request.requestId, sessionEpoch: 4, tabId: tab.id, operation: 'update-reload', outcomes: [{ name: 'alpha', status: 'partial', error: 'Saved on disk, but the current chat was not reloaded' }], loadedSkillNames: ['alpha'] }))
    expect(useKoma.getState().ui.toast?.text).toContain('Saved on disk')
    expect(useKoma.getState().ui.toast?.kind).toBe('warn')
    expect(screen.queryByText('Saved and reloaded in this chat.')).toBeNull()
  })

  it('reports unavailable IPC immediately as a definite unsent Save', () => {
    useKoma.setState((state) => ({ session: { ...state.session, id: 'session-a' }, skills: [entry] }))
    render(<SkillTab tab={tab} />)
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(useKoma.getState().ui.toast?.text).toContain('save was not sent')
    expect(useKoma.getState().ui.toast?.kind).toBe('error')
    expect(screen.getByRole('button', { name: 'Save' }).querySelector('[aria-hidden]')).toBeNull()
  })

  it('keeps a global skill editable and disables Project scope when no project is open', () => {
    const createTab = { id: 'skill:new', kind: 'skill' as const, skillId: null, title: 'New skill' }
    useKoma.setState((state) => ({ session: { ...state.session, id: 'session-a' }, skills: [entry], settingsValues: null }))
    const edit = render(<SkillTab tab={tab} />)
    expect(edit.getByRole('button', { name: 'Save' })).toHaveProperty('disabled', false)
    edit.unmount()

    const create = render(<SkillTab tab={createTab} />)
    fireEvent.change(create.getByPlaceholderText('e.g. review-notes'), { target: { value: 'new-skill' } })
    fireEvent.change(create.getByPlaceholderText('required — shown in the skill catalogue'), { target: { value: 'Does a thing' } })
    expect(create.getByRole('button', { name: 'Save' })).toHaveProperty('disabled', true)
    expect(create.getByText(/Open a project to add a skill/)).toBeTruthy()
    fireEvent.click(create.getByRole('button', { name: 'Global' }))
    expect(create.getByRole('button', { name: 'Project' })).toHaveProperty('disabled', true)
  })

  it('allows Project scope while creating inside an open project', () => {
    const createTab = { id: 'skill:new', kind: 'skill' as const, skillId: null, title: 'New skill' }
    useKoma.setState((state) => ({
      session: { ...state.session, id: 'session-a' },
      settingsValues: {
        name: 'test', workdir: ['/work'], shortSend: false, slidingCache: false, bashSaving: false,
        codingAutosave: false, internetMode: 'simple', palette: 'dark', effort: '', subagentMaxTurns: 500,
        shortSendEngageN: 0, shortSendTailN: 0, maxOutputTokens: 0, contextWindowLimit: 0,
        contextModelAlias: '', extraSkillRoots: [],
      },
      coding: { ...state.coding, activeRoot: '/work' },
    }))
    render(<SkillTab tab={createTab} />)
    fireEvent.change(screen.getByPlaceholderText('e.g. review-notes'), { target: { value: 'new-skill' } })
    fireEvent.change(screen.getByPlaceholderText('required — shown in the skill catalogue'), { target: { value: 'Does a thing' } })
    expect(screen.getByRole('button', { name: 'Save' })).toHaveProperty('disabled', false)
    fireEvent.click(screen.getByRole('button', { name: 'Global' }))
    expect(screen.getByRole('button', { name: 'Project' })).toHaveProperty('disabled', false)
  })

  it('renders lazy companion Markdown in Preview mode', () => {
    const companionDetail = { ...detail, companionFiles: ['references/guide.md'] }
    useKoma.setState({
      skills: [entry],
      skillDetails: { [tab.id]: companionDetail },
      skillFiles: { [`${tab.id}:references/guide.md`]: { content: '# Companion heading', error: null } },
    })
    render(<SkillTab tab={tab} />)
    fireEvent.click(screen.getByRole('tab', { name: 'references/guide.md' }))
    fireEvent.click(screen.getByRole('button', { name: 'Preview' }))
    expect(screen.getByTestId('markdown-preview').textContent).toContain('Companion heading')
  })
})
