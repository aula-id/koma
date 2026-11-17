import { describe, expect, it } from 'vitest'
import { repoOptionLabel } from './repoLabel'

describe('repoOptionLabel', () => {
  it('labels the workspace repo', () => {
    expect(repoOptionLabel({ root: '/w', name: 'w', rel: '.', kind: 'repo' })).toBe('w (workspace)')
  })
  it('treats entries from older hosts as the workspace repo', () => {
    expect(repoOptionLabel({ root: '/w', name: 'w' })).toBe('w (workspace)')
  })
  it('uses the relative path for nested repos', () => {
    expect(repoOptionLabel({ root: '/w/services/api', name: 'api', rel: 'services/api', kind: 'repo' })).toBe('services/api')
  })
  it('suffixes submodules and worktrees', () => {
    expect(repoOptionLabel({ root: '/w/m', name: 'm', rel: 'm', kind: 'submodule' })).toBe('m (submodule)')
    expect(repoOptionLabel({ root: '/w/x', name: 'x', rel: 'x', kind: 'worktree' })).toBe('x (worktree)')
    expect(repoOptionLabel({ root: '/w', name: 'w', rel: '.', kind: 'worktree' })).toBe('w (workspace) (worktree)')
  })
})
