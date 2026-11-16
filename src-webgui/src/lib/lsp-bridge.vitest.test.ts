import { expect, it } from 'vitest'
import { foldDrive, pathToUri, splitRootPath, uriToPath } from './lsp-bridge'

it('uriToPath folds the drive letter and keeps unix and UNC paths', () => {
  expect(uriToPath('file:///c%3A/Users/x/a.ts')).toBe('C:/Users/x/a.ts')
  expect(uriToPath('file:///C:/Users/x/a.ts')).toBe('C:/Users/x/a.ts')
  expect(uriToPath('file:///home/u/a.ts')).toBe('/home/u/a.ts')
  expect(uriToPath('file://srv/share/a.ts')).toBe('//srv/share/a.ts')
})

it('splitRootPath matches roots across drive-letter case and slash style', () => {
  expect(splitRootPath('c:/Users/x/src/a.ts', ['C:\\Users\\x'])).toEqual({ root: 'C:\\Users\\x', path: 'src/a.ts' })
  expect(splitRootPath('C:/Users/x/a.ts', ['c:\\Users\\x\\'])).toEqual({ root: 'c:\\Users\\x\\', path: 'a.ts' })
  expect(splitRootPath('C:/Users/x', ['C:\\Users\\x'])).toEqual({ root: 'C:\\Users\\x', path: '' })
  expect(splitRootPath('C:/Users/x/a.ts', ['\\\\?\\C:\\Users\\x'])).toEqual({ root: '\\\\?\\C:\\Users\\x', path: 'a.ts' })
  expect(splitRootPath('/users/x/a', ['/Users/x'])).toBeNull()
  expect(splitRootPath('/w/a/b/f.ts', ['/w', '/w/a'])).toEqual({ root: '/w/a', path: 'b/f.ts' })
})

it('pathToUri handles backslash roots and verbatim prefixes', () => {
  expect(pathToUri('C:\\Users\\x\\', 'src/a b.ts')).toBe('file:///C:/Users/x/src/a%20b.ts')
  expect(pathToUri('\\\\?\\C:\\Users\\x', 'a.ts')).toBe('file:///C:/Users/x/a.ts')
  expect(pathToUri('/home/x', 'a.ts')).toBe('file:///home/x/a.ts')
})

it('foldDrive only touches a leading drive letter', () => {
  expect(foldDrive('c:/a')).toBe('C:/a')
  expect(foldDrive('/c:/a')).toBe('/c:/a')
  expect(foldDrive('abc')).toBe('abc')
})
