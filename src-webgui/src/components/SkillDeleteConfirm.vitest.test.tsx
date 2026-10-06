import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useKoma, type SkillCatalogueEntry } from '../store/koma'
import { SkillDeleteConfirm } from './SkillDeleteConfirm'

const first: SkillCatalogueEntry = {
  skillId: 'source-a', generation: 'gen-a', name: 'review', description: '', triggers: '',
  sourceTier: 'global', scope: 'global', sourcePath: '/global/review/SKILL.md',
}
const second: SkillCatalogueEntry = {
  ...first, skillId: 'source-b', generation: 'gen-b', sourcePath: '/project/review/SKILL.md',
}

beforeEach(() => {
  window.ipc = { postMessage: vi.fn() }
  useKoma.setState((state) => ({
    skillSessionEpoch: 4, skillDeletePending: {}, skillOpResults: {}, skillLastOp: null,
    ui: {
      ...state.ui,
      tabs: [
        { id: 'chat', kind: 'chat' },
        { id: 'skill:source-a', kind: 'skill', skillId: 'source-a', title: 'review' },
        { id: 'skill:source-b', kind: 'skill', skillId: 'source-b', title: 'review' },
      ],
      activeTabId: 'skill:source-a', groups: ['g0'], tabGroup: { chat: 'g0', 'skill:source-a': 'g0', 'skill:source-b': 'g0' },
      groupActive: { g0: 'skill:source-a' }, activeGroupId: 'g0',
    },
  }))
})
afterEach(() => { cleanup(); delete window.ipc })

it('keeps tabs until confirmation, then closes only successfully deleted sources in a bulk operation', () => {
  render(<SkillDeleteConfirm skills={[first, second]} onClose={vi.fn()} />)
  fireEvent.click(screen.getByRole('button', { name: 'Delete forever' }))
  const calls = vi.mocked(window.ipc!.postMessage).mock.calls
  const request = JSON.parse(calls[calls.length - 1][0])
  expect(request).toMatchObject({ r: 'DeleteSkills', sessionEpoch: 4, tabId: 'delete-dialog' })
  expect(request.items).toEqual([
    { skillId: first.skillId, generation: first.generation, name: first.name },
    { skillId: second.skillId, generation: second.generation, name: second.name },
  ])
  expect(useKoma.getState().ui.tabs).toHaveLength(3)

  act(() => useKoma.getState().push({
    k: 'SkillOp', requestId: request.requestId, sessionEpoch: 4, tabId: 'delete-dialog', operation: 'delete',
    outcomes: [{ name: 'review', status: 'success' }, { name: 'review', status: 'failed', error: 'source changed' }],
    loadedSkillNames: [],
  }))
  expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toEqual(['chat', 'skill:source-b'])
  expect(screen.getByText('Deleted')).toBeTruthy()
  expect(screen.getByText('source changed')).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Close' })).toBeTruthy()
})

it('does not close the editor after a failed Delete', () => {
  render(<SkillDeleteConfirm skills={[first]} onClose={vi.fn()} />)
  fireEvent.click(screen.getByRole('button', { name: 'Delete forever' }))
  const calls = vi.mocked(window.ipc!.postMessage).mock.calls
  const request = JSON.parse(calls[calls.length - 1][0])
  act(() => useKoma.getState().push({
    k: 'SkillOp', requestId: request.requestId, sessionEpoch: 4, tabId: 'delete-dialog', operation: 'delete',
    outcomes: [{ name: 'review', status: 'failed', error: 'stale generation' }], loadedSkillNames: [],
  }))
  expect(useKoma.getState().ui.tabs.map((tab) => tab.id)).toContain('skill:source-a')
  expect(screen.getByText('stale generation')).toBeTruthy()
})
