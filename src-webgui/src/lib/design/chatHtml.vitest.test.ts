import { describe, expect, it } from 'vitest'
import { designFileUrl } from './chatHtml'

describe('designFileUrl', () => {
  it('keeps POSIX output', () => {
    expect(designFileUrl('/a/b', './c.png')).toBe('file:///a/b/c.png')
    expect(designFileUrl('/a/b/', 'x/c d.png')).toBe('file:///a/b/x/c%20d.png')
    expect(designFileUrl('/a/b', '/abs/c.png')).toBe('file:///abs/c.png')
  })
  it('builds drive-letter URLs from Windows roots', () => {
    expect(designFileUrl('C:\\Users\\me\\proj', 'assets/a b.png')).toBe(
      'file:///C:/Users/me/proj/assets/a%20b.png',
    )
    expect(designFileUrl('\\\\?\\C:\\Users\\me\\proj\\', 'a.png')).toBe('file:///C:/Users/me/proj/a.png')
    expect(designFileUrl('C:\\p', 'D:\\other\\a.png')).toBe('file:///D:/other/a.png')
  })
})
