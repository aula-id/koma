import { afterEach, expect, it } from 'vitest'
import { render } from 'vitest-browser-react'
import { LexicalMarkdownEditor } from './LexicalMarkdownEditor'
import '../../styles.css'

const roles = ['fg', 'bg', 'accent', 'dim'] as const
const saved = roles.map((role) => document.documentElement.style.getPropertyValue(`--koma-${role}`))

afterEach(() => {
  roles.forEach((role, index) => {
    if (saved[index]) document.documentElement.style.setProperty(`--koma-${role}`, saved[index])
    else document.documentElement.style.removeProperty(`--koma-${role}`)
  })
})

it('updates rich text, bullets, numbers, checkboxes and selection when the palette changes', async () => {
  await render(<LexicalMarkdownEditor
    profile="composer"
    markdown={'# Heading\n\nPlain `code` [link](https://example.com)\n\n> Quote\n\n- Bullet\n    - Nested\n\n1. Number\n2. Another\n\n- [x] Checked'}
    onMarkdown={() => {}}
  />)
  const editor = document.querySelector<HTMLElement>('.koma-rich-editor')!
  await expect.poll(() => editor.querySelector('h1')?.textContent).toBe('Heading')
  for (const palette of [
    { fg: '#c8d3f5', bg: '#0b0e14', accent: '#39ff14', dim: '#adadad' },
    { fg: '#202020', bg: '#fafafa', accent: '#2555aa', dim: '#555555' },
  ]) {
    roles.forEach((role) => document.documentElement.style.setProperty(`--koma-${role}`, palette[role]))
    const probe = document.createElement('span')
    editor.append(probe)
    const color = (hex: string) => {
      probe.style.color = hex
      return getComputedStyle(probe).color
    }
    expect(getComputedStyle(editor).color).toBe(color(palette.fg))
    expect(getComputedStyle(editor).caretColor).toBe(color(palette.fg))
    for (const selector of ['h1', 'a', 'span.text-koma-accent']) {
      expect(editor.querySelector(selector), selector).not.toBeNull()
      expect(getComputedStyle(editor.querySelector(selector)!).color).toBe(color(palette.accent))
    }
    expect(getComputedStyle(editor.querySelector('blockquote')!).color).toBe(color(palette.dim))
    for (const selector of ['ul.list-disc > li', 'ol > li']) {
      const item = editor.querySelector(selector)!
      expect(getComputedStyle(item, '::marker').color).toBe(color(palette.fg))
    }
    expect(getComputedStyle(editor.querySelector('ul.list-disc')!).listStyleType).toBe('disc')
    const ordered = editor.querySelector('ol')!
    expect(getComputedStyle(ordered).listStyleType).toBe('decimal')
    expect(parseFloat(getComputedStyle(ordered).paddingLeft)).toBeGreaterThanOrEqual(24)
    const nested = [...editor.querySelectorAll('li')].find((item) => item.firstElementChild?.tagName === 'UL')!
    expect(getComputedStyle(nested).listStyleType).toBe('none')
    expect(getComputedStyle(editor.querySelector('.koma-checklist-checked')!, '::before').backgroundColor).toBe(color(palette.accent))
    expect(getComputedStyle(editor, '::selection').color).toBe(color(palette.fg))
    const selectionBackground = getComputedStyle(editor, '::selection').backgroundColor
    expect(selectionBackground).not.toBe('rgba(0, 0, 0, 0)')
    probe.remove()
  }
})

it('keeps ordered-list numbers inside the composer overflow box', async () => {
  await render(<LexicalMarkdownEditor
    profile="composer"
    markdown={'1. Number\n2. Another'}
    onMarkdown={() => {}}
    className="relative z-0 max-h-[200px] min-h-[22px] overflow-y-auto m-0 box-border w-full p-0 text-[14px] leading-[22px]"
  />)
  const editor = document.querySelector<HTMLElement>('.koma-rich-editor')!
  await expect.poll(() => editor.querySelector('ol')?.querySelector('li')?.textContent).toBe('Number')
  const wrap = editor.closest('[data-composer-editor]')!
  const ordered = editor.querySelector('ol')!
  const item = ordered.querySelector('li')!
  expect(getComputedStyle(ordered).listStyleType).toBe('decimal')
  expect(getComputedStyle(item).listStyleType).not.toBe('none')
  const pad = parseFloat(getComputedStyle(ordered).paddingLeft)
  expect(pad).toBeGreaterThanOrEqual(24)
  const wrapRect = wrap.getBoundingClientRect()
  const itemRect = item.getBoundingClientRect()
  expect(itemRect.left - wrapRect.left).toBeGreaterThanOrEqual(pad - 1)
})
