import { expect, it } from 'vitest'
import { executionMatches } from './coding-debug'

it('matches Windows paths across drive case, slash style and case', () => {
  expect(executionMatches('c:\\proj', 'src/a.ts', 'C:/proj/src/a.ts')).toBe(true)
  expect(executionMatches('\\\\?\\C:\\proj', 'src/a.ts', 'C:/Proj/SRC/a.ts')).toBe(true)
  expect(executionMatches('C:\\proj', 'src/b.ts', 'C:/proj/src/a.ts')).toBe(false)
})

it('POSIX stays case-sensitive', () => {
  expect(executionMatches('/w', 'src/a.ts', '/w/src/a.ts')).toBe(true)
  expect(executionMatches('/w', 'src/a.ts', '/w/SRC/a.ts')).toBe(false)
  expect(executionMatches('/w', 'a\\b.ts', '/w/a\\b.ts')).toBe(true)
})
