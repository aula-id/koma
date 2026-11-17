import { describe, expect, it } from 'vitest'
import { fileUrl, pathBasename, stripVerbatim } from './pathDisplay'

describe('pathBasename', () => {
  it('handles both separator styles', () => {
    expect(pathBasename('C:\\Users\\x\\proj')).toBe('proj')
    expect(pathBasename('C:\\Users\\x\\proj\\')).toBe('proj')
    expect(pathBasename('\\\\?\\C:\\x\\proj')).toBe('proj')
    expect(pathBasename('/home/a/b')).toBe('b')
  })
  it('falls back to the input when there is no component', () => {
    expect(pathBasename('/')).toBe('/')
    expect(pathBasename('C:\\')).toBe('C:')
  })
})

describe('stripVerbatim', () => {
  it('strips drive and UNC prefixes only', () => {
    expect(stripVerbatim('\\\\?\\C:\\x')).toBe('C:\\x')
    expect(stripVerbatim('//?/C:/x')).toBe('C:/x')
    expect(stripVerbatim('\\\\?\\UNC\\srv\\share\\d')).toBe('\\\\srv\\share\\d')
    expect(stripVerbatim('\\\\?\\Volume{1}\\x')).toBe('\\\\?\\Volume{1}\\x')
    expect(stripVerbatim('/usr/x')).toBe('/usr/x')
  })
})

describe('fileUrl', () => {
  it('builds file URLs for every path style', () => {
    expect(fileUrl('C:\\a b\\x.png')).toBe('file:///C:/a%20b/x.png')
    expect(fileUrl('\\\\?\\C:\\a\\x.png')).toBe('file:///C:/a/x.png')
    expect(fileUrl('\\\\srv\\share\\x')).toBe('file://srv/share/x')
    expect(fileUrl('/home/a/x.png')).toBe('file:///home/a/x.png')
    expect(fileUrl('/home/a/b\\c.png')).toBe('file:///home/a/b%5Cc.png')
  })
})

import { isWindowsPath, pathKey, pathsEqual, relativeUnder, rootLabel, toSlashIfWindows } from './pathDisplay'

describe('platform-gated path helpers', () => {
  it('detects Windows shapes only', () => {
    for (const p of ['C:\\x', 'c:/x', '\\\\?\\C:\\x', '\\\\srv\\sh']) expect(isWindowsPath(p)).toBe(true)
    for (const p of ['/home/x', 'a\\b', './x']) expect(isWindowsPath(p)).toBe(false)
  })
  it('keeps backslashes in POSIX paths', () => {
    expect(toSlashIfWindows('/home/u/a\\b.ts')).toBe('/home/u/a\\b.ts')
    expect(toSlashIfWindows('C:\\u\\a.ts')).toBe('C:/u/a.ts')
    expect(pathKey('/home/u/a\\b/')).toBe('/home/u/a\\b')
  })
  it('folds case only for Windows', () => {
    expect(pathsEqual('c:\\Proj', 'C:/proj/')).toBe(true)
    expect(pathsEqual('/Home/x', '/home/x')).toBe(false)
  })
  it('relativeUnder is boundary safe and keeps tail case', () => {
    expect(relativeUnder('C:\\proj', 'c:/PROJ/src/a.ts')).toBe('src/a.ts')
    expect(relativeUnder('\\\\?\\C:\\proj', 'C:/proj/Src/A.ts')).toBe('Src/A.ts')
    expect(relativeUnder('/w', '/w-secret/x')).toBeNull()
    expect(relativeUnder('/w', '/w')).toBe('')
    expect(relativeUnder('/w', '/W/x')).toBeNull()
    expect(relativeUnder('/w', '/w/a\\b.ts')).toBe('a\\b.ts')
  })
  it('rootLabel handles both separators', () => {
    expect(rootLabel('C:\\Users\\me\\proj')).toBe('proj')
    expect(rootLabel('/a/b')).toBe('b')
  })
})
