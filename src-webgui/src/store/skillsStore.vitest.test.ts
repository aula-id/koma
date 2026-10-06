import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useKoma } from './koma'

const skill = {
  skillId: 'opaque-a', generation: 'gen-a', name: 'alpha', description: 'A', triggers: '',
  sourceTier: 'global', scope: 'global' as const, sourcePath: '/tmp/alpha/SKILL.md',
}

beforeEach(() => {
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => { cb(0); return 1 })
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
      preserveTabsOnNextSession: false,
      preservedTabLayout: null,
      preservedTabsTargetSession: null,
    },
    skills: [], loadedSkillNames: [], skillsLoading: false, skillsError: null, skillsUnconfirmed: null,
    skillRequestId: null, skillSessionEpoch: 4, skillSelection: [], skillDetails: {},
    skillDetailPending: {}, skillDetailErrors: {}, skillFiles: {}, skillOutcomes: [], skillLastOp: null,
    skillDeletePending: {}, skillOpResults: {},
    skillQuery: 'persist me', skillFilter: 'global',
  }))
})

function attachSnapshot(session: string, loadedSkillNames: string[] = []) {
  useKoma.getState().push({
    k: 'Snapshot', session, state: 'idle', messages: [], title: '', subagents: [], bash: [],
    fileChanges: [], attachments: [], pendingSteer: [], awaitingApproval: false,
    palette: useKoma.getState().palette, loadedSkillNames,
  } as any)
}

describe('Skills store correlation', () => {
  it('rejects catalogue replies from a previous session epoch', () => {
    useKoma.getState().push({ k: 'SkillValues', requestId: 'old', sessionEpoch: 3, skills: [skill], loadedSkillNames: ['alpha'], error: null })
    expect(useKoma.getState().skills).toEqual([])
    expect(useKoma.getState().loadedSkillNames).toEqual([])
  })

  it('keeps only the foreground Loaded set through A→B→A and ignores late catalogue replies', () => {
    const beta = { ...skill, skillId: 'opaque-b', generation: 'gen-b', name: 'beta' }
    attachSnapshot('session-a', ['alpha'])
    const epochA = useKoma.getState().skillSessionEpoch
    useKoma.setState({ skills: [skill], skillSelection: [skill.skillId] })
    attachSnapshot('session-b', ['beta'])
    const epochB = useKoma.getState().skillSessionEpoch
    expect(epochB).toBe(epochA + 1)
    expect(useKoma.getState().skills).toEqual([])
    expect(useKoma.getState().skillSelection).toEqual([])
    expect(useKoma.getState().loadedSkillNames).toEqual(['beta'])
    useKoma.getState().push({ k: 'SkillValues', requestId: 'stale-a', sessionEpoch: epochA, skills: [skill], loadedSkillNames: ['alpha'], error: null })
    expect(useKoma.getState().skills).toEqual([])

    useKoma.getState().refreshSkills()
    const requestB = useKoma.getState().skillRequestId!
    useKoma.getState().push({ k: 'SkillValues', requestId: requestB, sessionEpoch: epochB, skills: [beta], loadedSkillNames: ['beta'], error: null })
    expect(useKoma.getState().skills).toEqual([beta])
    attachSnapshot('session-b', ['beta'])
    expect(useKoma.getState().skillSessionEpoch).toBe(epochB)
    expect(useKoma.getState().skills).toEqual([beta])

    attachSnapshot('session-a', ['alpha'])
    const nextEpochA = useKoma.getState().skillSessionEpoch
    expect(nextEpochA).toBe(epochB + 1)
    expect(useKoma.getState().loadedSkillNames).toEqual(['alpha'])
    expect(useKoma.getState().skills).toEqual([])
    useKoma.getState().push({ k: 'SkillValues', requestId: requestB, sessionEpoch: epochB, skills: [beta], loadedSkillNames: ['beta'], error: null })
    expect(useKoma.getState().skills).toEqual([])
    useKoma.getState().refreshSkills()
    const requestA = useKoma.getState().skillRequestId!
    useKoma.getState().push({ k: 'SkillValues', requestId: requestA, sessionEpoch: nextEpochA, skills: [skill], loadedSkillNames: ['alpha'], error: null })
    expect(useKoma.getState().skills).toEqual([skill])
    expect(useKoma.getState().loadedSkillNames).toEqual(['alpha'])
  })

  it('bounds a missing Rescan response without treating it as a failed disk operation', () => {
    vi.useFakeTimers()
    try {
      useKoma.setState({ skills: [skill] })
      useKoma.getState().refreshSkills()
      const first = useKoma.getState().skillRequestId!
      expect(useKoma.getState().skillsLoading).toBe(true)
      vi.advanceTimersByTime(12_001)
      expect(useKoma.getState().skillsLoading).toBe(false)
      expect(useKoma.getState().skillsUnconfirmed).toContain('not confirmed')
      expect(useKoma.getState().skills).toEqual([skill])
      useKoma.getState().refreshSkills()
      const second = useKoma.getState().skillRequestId!
      expect(second).not.toBe(first)
      useKoma.getState().push({ k: 'SkillValues', requestId: first, sessionEpoch: 4, skills: [], loadedSkillNames: [], error: null })
      expect(useKoma.getState().skillsLoading).toBe(true)
      useKoma.getState().push({ k: 'SkillValues', requestId: second, sessionEpoch: 4, skills: [skill], loadedSkillNames: [], error: null })
      expect(useKoma.getState().skillsLoading).toBe(false)
      expect(useKoma.getState().skillsUnconfirmed).toBeNull()
    } finally { vi.useRealTimers() }
  })

  it('accepts a late catalogue response even after its spinner is stopped', () => {
    vi.useFakeTimers()
    try {
      useKoma.getState().refreshSkills()
      const requestId = useKoma.getState().skillRequestId!
      vi.advanceTimersByTime(12_001)
      useKoma.getState().push({ k: 'SkillValues', requestId, sessionEpoch: 4, skills: [skill], loadedSkillNames: [], error: null })
      expect(useKoma.getState().skills).toEqual([skill])
      expect(useKoma.getState().skillsUnconfirmed).toBeNull()
    } finally { vi.useRealTimers() }
  })

  it('bounds a missing catalogue follow-up after a filesystem operation', () => {
    vi.useFakeTimers()
    try {
      useKoma.getState().push({ k: 'SkillOp', requestId: 'update-no-followup', sessionEpoch: 4, tabId: 'skill:a', operation: 'update', outcomes: [{ name: 'alpha', status: 'success' }], loadedSkillNames: [] })
      expect(useKoma.getState().skillsLoading).toBe(true)
      vi.advanceTimersByTime(12_001)
      expect(useKoma.getState().skillsLoading).toBe(false)
      expect(useKoma.getState().skillOpResults['update-no-followup']?.outcomes[0].status).toBe('success')
      expect(useKoma.getState().skillsUnconfirmed).toContain('not confirmed')
    } finally { vi.useRealTimers() }
  })

  it('accepts the current correlated catalogue as an authoritative replacement', () => {
    useKoma.setState({ skillRequestId: 'current', skillsLoading: true, loadedSkillNames: ['stale'] })
    useKoma.getState().push({ k: 'SkillValues', requestId: 'current', sessionEpoch: 4, skills: [skill], loadedSkillNames: ['alpha'], error: null })
    expect(useKoma.getState().skills).toEqual([skill])
    expect(useKoma.getState().loadedSkillNames).toEqual(['alpha'])
    expect(useKoma.getState().skillsLoading).toBe(false)
    useKoma.getState().push({ k: 'SkillValues', requestId: 'older', sessionEpoch: 4, skills: [], loadedSkillNames: [], error: null })
    expect(useKoma.getState().skills).toEqual([skill])
  })

  it('does not wait for a catalogue reply after Load/Unload/Reload context changes', () => {
    useKoma.getState().push({ k: 'SkillOp', requestId: 'load-1', sessionEpoch: 4, tabId: 'skills-panel', operation: 'load', outcomes: [{ name: 'alpha', status: 'success' }], loadedSkillNames: ['alpha'] })
    expect(useKoma.getState().loadedSkillNames).toEqual(['alpha'])
    expect(useKoma.getState().skillRequestId).toBeNull()
    expect(useKoma.getState().skillsLoading).toBe(false)
  })

  it('correlates the authoritative catalogue refresh after a ZIP install', () => {
    useKoma.setState({ skillRequestId: 'previous' })
    useKoma.getState().push({ k: 'SkillOp', requestId: 'upload-1', sessionEpoch: 4, tabId: 'upload-skill', operation: 'install-zip', outcomes: [{ name: 'alpha', status: 'success' }], loadedSkillNames: [] })
    expect(useKoma.getState().skillRequestId).toBe('upload-1')
    expect(useKoma.getState().skillsLoading).toBe(true)
    useKoma.getState().push({ k: 'SkillValues', requestId: 'upload-1', sessionEpoch: 4, skills: [skill], loadedSkillNames: [], error: null })
    expect(useKoma.getState().skills).toEqual([skill])
  })

  it('uses SkillOp correlation for the catalogue refresh that follows a mutation', () => {
    useKoma.getState().push({ k: 'SkillOp', requestId: 'save-1', sessionEpoch: 4, tabId: 'skill:a', operation: 'update', outcomes: [{ name: 'alpha', status: 'success' }], loadedSkillNames: [] })
    expect(useKoma.getState().skillRequestId).toBe('save-1')
    useKoma.getState().push({ k: 'SkillValues', requestId: 'different', sessionEpoch: 4, skills: [skill], loadedSkillNames: [], error: null })
    expect(useKoma.getState().skills).toEqual([])
    useKoma.getState().push({ k: 'SkillValues', requestId: 'save-1', sessionEpoch: 4, skills: [skill], loadedSkillNames: [], error: null })
    expect(useKoma.getState().skills).toEqual([skill])
  })

  it('correlates Save & Reload with the subsequent catalogue replacement', () => {
    useKoma.getState().push({ k: 'SkillOp', requestId: 'save-reload-1', sessionEpoch: 4, tabId: 'skill:a', operation: 'update-reload', outcomes: [{ name: 'alpha', status: 'partial', detail: 'Disk saved, chat unchanged' }], loadedSkillNames: ['alpha'] })
    expect(useKoma.getState().skillRequestId).toBe('save-reload-1')
    expect(useKoma.getState().skillsLoading).toBe(true)
    expect(useKoma.getState().skillOpResults['save-reload-1']?.outcomes[0].status).toBe('partial')
    useKoma.getState().push({ k: 'SkillValues', requestId: 'save-reload-1', sessionEpoch: 4, skills: [skill], loadedSkillNames: ['alpha'], error: null })
    expect(useKoma.getState().skills).toEqual([skill])
    expect(useKoma.getState().skillsLoading).toBe(false)
  })

  it('closes only confirmed deleted source tabs, by request and input order rather than name', () => {
    const second = { ...skill, skillId: 'opaque-b', generation: 'gen-b' }
    useKoma.setState((state) => ({ ui: {
      ...state.ui,
      tabs: [
        { id: 'chat', kind: 'chat' },
        { id: 'skill:opaque-a', kind: 'skill', skillId: 'opaque-a', title: 'alpha' },
        { id: 'skill:opaque-b', kind: 'skill', skillId: 'opaque-b', title: 'alpha' },
      ],
    } }))
    useKoma.getState().registerSkillDelete('delete-1', 4, [skill, second])
    useKoma.getState().push({ k: 'SkillOp', requestId: 'unrelated', sessionEpoch: 4, tabId: 'delete-dialog', operation: 'delete', outcomes: [{ name: 'alpha', status: 'success' }], loadedSkillNames: [] })
    expect(useKoma.getState().ui.tabs).toHaveLength(3)
    useKoma.getState().push({ k: 'SkillOp', requestId: 'delete-1', sessionEpoch: 3, tabId: 'delete-dialog', operation: 'delete', outcomes: [{ name: 'alpha', status: 'success' }, { name: 'alpha', status: 'success' }], loadedSkillNames: [] })
    expect(useKoma.getState().ui.tabs).toHaveLength(3)
    useKoma.getState().push({ k: 'SkillOp', requestId: 'delete-1', sessionEpoch: 4, tabId: 'delete-dialog', operation: 'delete', outcomes: [{ name: 'alpha', status: 'success' }, { name: 'alpha', status: 'failed', error: 'changed on disk' }], loadedSkillNames: [] })
    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual(['chat', 'skill:opaque-b'])
    expect(useKoma.getState().skillDeletePending['delete-1']).toBeUndefined()
    // A replay must not close a tab that was not successfully deleted.
    useKoma.getState().push({ k: 'SkillOp', requestId: 'delete-1', sessionEpoch: 4, tabId: 'delete-dialog', operation: 'delete', outcomes: [{ name: 'alpha', status: 'success' }, { name: 'alpha', status: 'success' }], loadedSkillNames: [] })
    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual(['chat', 'skill:opaque-b'])
  })

  it('preserves skill and upload tabs when the first session attaches from detached state', () => {
    useKoma.setState((state) => ({
      ui: {
        ...state.ui,
        tabs: [
          { id: 'chat', kind: 'chat' },
          { id: 'skill:opaque-a', kind: 'skill', skillId: 'opaque-a', title: 'alpha' },
          { id: 'skill:opaque-b', kind: 'skill', skillId: 'opaque-b', title: 'beta' },
          { id: 'upload-skill', kind: 'uploadSkill' },
        ],
        tabGroup: { chat: 'g0', 'skill:opaque-a': 'g0', 'skill:opaque-b': 'g0', 'upload-skill': 'g0' },
        groupActive: { g0: 'chat' },
      },
    }))

    attachSnapshot('session-a')

    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual([
      'chat', 'skill:opaque-a', 'skill:opaque-b', 'upload-skill',
    ])
  })

  it('re-discovers retained Skill tabs only when the guided target Snapshot arrives', () => {
    const sent: Array<{ r: string; requestId?: string; sessionEpoch?: number }> = []
    const previousIpc = window.ipc
    window.ipc = { postMessage: (json: string) => sent.push(JSON.parse(json)) }
    try {
      useKoma.setState((state) => ({
        skills: [skill],
        ui: {
          ...state.ui,
          tabs: [{ id: 'chat', kind: 'chat' }, { id: 'skill:opaque-a', kind: 'skill', skillId: 'opaque-a', title: 'alpha' }],
          tabGroup: { chat: 'g0', 'skill:opaque-a': 'g0' },
        },
      }))
      useKoma.getState().newSessionPreservingTabs()
      useKoma.getState().refillComposer('Help me edit alpha')
      useKoma.getState().push({ k: 'Switching', to: 'guided-target' })
      attachSnapshot('older-in-flight')
      expect(sent.filter((request) => request.r === 'GetSkills')).toHaveLength(0)
      expect(useKoma.getState().ui.composerRefill).toBe('Help me edit alpha')

      attachSnapshot('guided-target')
      const discoveries = sent.filter((request) => request.r === 'GetSkills')
      expect(discoveries).toHaveLength(1)
      expect(discoveries[0].sessionEpoch).toBe(useKoma.getState().skillSessionEpoch)
      expect(useKoma.getState().skillsLoading).toBe(true)
      expect(useKoma.getState().ui.composerRefill).toBe('Help me edit alpha')
      useKoma.getState().push({ k: 'SkillValues', requestId: discoveries[0].requestId!, sessionEpoch: discoveries[0].sessionEpoch!, skills: [skill], loadedSkillNames: [], error: null })
      expect(useKoma.getState().skills).toEqual([skill])
      expect(useKoma.getState().skillsLoading).toBe(false)
    } finally {
      window.ipc = previousIpc
    }
  })

  it('restores preview.3 nested split layout for a guided first chat', () => {
    window.ipc = { postMessage: vi.fn() }
    useKoma.setState((state) => ({ ui: {
      ...state.ui,
      tabs: [
        { id: 'chat', kind: 'chat' },
        { id: 'skill:opaque-a', kind: 'skill', skillId: 'opaque-a', title: 'alpha' },
      ],
      groups: ['g0', 'g1'],
      tabGroup: { chat: 'g0', 'skill:opaque-a': 'g1' },
      groupActive: { g0: 'chat', g1: 'skill:opaque-a' },
      activeGroupId: 'g1',
      activeTabId: 'skill:opaque-a',
      splitTree: { type: 'split', id: 's0', dir: 'row', aSize: 0.6, bSize: 0.4,
        a: { type: 'leaf', id: 'g0' }, b: { type: 'leaf', id: 'g1' } },
      groupSplitDir: { g0: 'row', g1: 'row' },
    } }))
    useKoma.getState().newSessionPreservingTabs()
    useKoma.getState().push({ k: 'Switching', to: 'guided-chat' })
    useKoma.getState().detachSession()
    attachSnapshot('guided-chat')
    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual(['chat', 'skill:opaque-a'])
    expect(useKoma.getState().ui.splitTree).toMatchObject({ type: 'split', id: 's0', aSize: 0.6, bSize: 0.4 })
    expect(useKoma.getState().ui.tabGroup['skill:opaque-a']).toBe('g1')
  })

  it('restores detached editor tabs even if an intermediate attach event resets the live layout', () => {
    window.ipc = { postMessage: vi.fn() }
    useKoma.setState((state) => ({
      ui: {
        ...state.ui,
        tabs: [
          { id: 'chat', kind: 'chat' },
          { id: 'skill:opaque-a', kind: 'skill', skillId: 'opaque-a', title: 'alpha' },
          { id: 'skill:opaque-b', kind: 'skill', skillId: 'opaque-b', title: 'beta' },
          { id: 'upload-skill', kind: 'uploadSkill' },
        ],
        activeTabId: 'skill:opaque-a',
        tabGroup: { chat: 'g0', 'skill:opaque-a': 'g0', 'skill:opaque-b': 'g0', 'upload-skill': 'g0' },
        groupActive: { g0: 'skill:opaque-a' },
      },
    }))

    useKoma.getState().newSessionPreservingTabs()
    useKoma.getState().push({ k: 'Switching', to: 'session-a' })
    expect(useKoma.getState().ui.preservedTabLayout?.tabs.map((tab) => tab.id)).toEqual([
      'chat', 'skill:opaque-a', 'skill:opaque-b', 'upload-skill',
    ])

    // Reproduce the real detached attach race: a host/workspace transition can
    // collapse the live layout before the authoritative first Snapshot arrives.
    // Preview.7's forced-close cleanup must not kill the captured guided tabs.
    const originalCloseAll = useKoma.getState().closeAllTabsExceptChat
    const closeAll = vi.fn(originalCloseAll)
    useKoma.setState({ closeAllTabsExceptChat: closeAll })
    useKoma.getState().detachSession()
    expect(closeAll).not.toHaveBeenCalled()
    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual(['chat'])

    attachSnapshot('session-a')
    useKoma.setState({ closeAllTabsExceptChat: originalCloseAll })

    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual([
      'chat', 'skill:opaque-a', 'skill:opaque-b', 'upload-skill',
    ])
    expect(useKoma.getState().ui.activeTabId).toBe('chat')
    expect(useKoma.getState().ui.preserveTabsOnNextSession).toBe(false)
    expect(useKoma.getState().ui.preservedTabLayout).toBeNull()
  })

  it('keeps all editor tabs across a stale Snapshot followed by the guided chat target', () => {
    window.ipc = { postMessage: vi.fn() }
    useKoma.setState((state) => ({
      ui: {
        ...state.ui,
        tabs: [
          { id: 'chat', kind: 'chat' },
          { id: 'skill:opaque-a', kind: 'skill', skillId: 'opaque-a', title: 'alpha' },
          { id: 'skill:opaque-b', kind: 'skill', skillId: 'opaque-b', title: 'beta' },
          { id: 'upload-skill', kind: 'uploadSkill' },
        ],
        activeTabId: 'skill:opaque-a',
        tabGroup: { chat: 'g0', 'skill:opaque-a': 'g0', 'skill:opaque-b': 'g0', 'upload-skill': 'g0' },
        groupActive: { g0: 'skill:opaque-a' },
      },
    }))

    useKoma.getState().newSessionPreservingTabs()
    useKoma.getState().push({ k: 'Switching', to: 'guided-chat' })
    attachSnapshot('stale-in-flight-chat')
    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual([
      'chat', 'skill:opaque-a', 'skill:opaque-b', 'upload-skill',
    ])
    expect(useKoma.getState().ui.preserveTabsOnNextSession).toBe(true)
    expect(useKoma.getState().ui.preservedTabsTargetSession).toBe('guided-chat')

    attachSnapshot('guided-chat')
    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual([
      'chat', 'skill:opaque-a', 'skill:opaque-b', 'upload-skill',
    ])
    expect(useKoma.getState().ui.activeTabId).toBe('chat')
    expect(useKoma.getState().ui.preserveTabsOnNextSession).toBe(false)
    expect(useKoma.getState().ui.preservedTabLayout).toBeNull()
    expect(useKoma.getState().ui.preservedTabsTargetSession).toBeNull()
  })

  it('honors and consumes explicit preservation across a replacement-style Snapshot', () => {
    useKoma.setState((state) => ({
      session: { ...state.session, id: 'session-a' },
      ui: {
        ...state.ui,
        preserveTabsOnNextSession: true,
        preservedTabsTargetSession: 'session-b',
        tabs: [
          { id: 'chat', kind: 'chat' },
          { id: 'skill:opaque-a', kind: 'skill', skillId: 'opaque-a', title: 'alpha' },
          { id: 'skill:opaque-b', kind: 'skill', skillId: 'opaque-b', title: 'beta' },
        ],
        tabGroup: { chat: 'g0', 'skill:opaque-a': 'g0', 'skill:opaque-b': 'g0' },
        groupActive: { g0: 'chat' },
      },
    }))

    const originalCloseAll = useKoma.getState().closeAllTabsExceptChat
    const closeAll = vi.fn(originalCloseAll)
    useKoma.setState({ closeAllTabsExceptChat: closeAll })
    attachSnapshot('session-b')

    expect(closeAll).not.toHaveBeenCalled()
    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual([
      'chat', 'skill:opaque-a', 'skill:opaque-b',
    ])
    expect(useKoma.getState().ui.preserveTabsOnNextSession).toBe(false)
    useKoma.setState({ closeAllTabsExceptChat: originalCloseAll })
  })

  it('still clears prior-session editor tabs when switching between attached sessions', () => {
    useKoma.setState((state) => ({
      session: { ...state.session, id: 'session-a' },
      ui: {
        ...state.ui,
        tabs: [
          { id: 'chat', kind: 'chat' },
          { id: 'skill:opaque-a', kind: 'skill', skillId: 'opaque-a', title: 'alpha' },
        ],
        activeTabId: 'skill:opaque-a',
        tabGroup: { chat: 'g0', 'skill:opaque-a': 'g0' },
        groupActive: { g0: 'skill:opaque-a' },
      },
    }))

    const originalCloseAll = useKoma.getState().closeAllTabsExceptChat
    const closeAll = vi.fn(originalCloseAll)
    useKoma.setState({ closeAllTabsExceptChat: closeAll })
    attachSnapshot('session-b')

    expect(closeAll).toHaveBeenCalledWith({ force: true })
    expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual(['chat'])
    expect(useKoma.getState().ui.activeTabId).toBe('chat')
    useKoma.setState({ closeAllTabsExceptChat: originalCloseAll })
  })
})
