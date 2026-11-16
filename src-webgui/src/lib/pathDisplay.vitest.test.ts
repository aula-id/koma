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
  })
})
