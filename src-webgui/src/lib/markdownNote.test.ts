import assert from 'node:assert/strict'
import { inlineFromHtml, inlineToHtml, parseNote, safeNoteUrl, serializeNote } from './markdownNote.ts'

{
  const source = ['# Title', '', 'Hello **there**', '', '- one', '- two', '', '![wire](img-1.png)', '', '```', 'const a = 1', '```'].join('\n')
  const blocks = parseNote(source)
  assert.deepEqual(blocks.map((block) => block.kind), ['h1', 'p', 'ul', 'image', 'code'])
  assert.equal(serializeNote(blocks), source)
}

assert.equal(inlineFromHtml(inlineToHtml('**bold** and *em* `code`')), '**bold** and *em* `code`')
assert.equal(inlineFromHtml(inlineToHtml('see [koma](https://koma.run)')), 'see [koma](https://koma.run)')
assert.equal(inlineFromHtml(inlineToHtml('a\nb')), 'a\nb')
assert.equal(inlineFromHtml('<b>x</b>'), '**x**')
assert.equal(safeNoteUrl('javascript:alert(1)'), false)
assert.equal(safeNoteUrl('https://koma.run/a'), true)
assert.equal(inlineToHtml('[x](javascript:alert(1))').includes('<a'), false)

{
  const blocks = parseNote('```\nkeep ```\ninside\n```')
  assert.equal(blocks[0]?.kind, 'code')
  if (blocks[0]?.kind === 'code') assert.equal(blocks[0].text.includes('```'), true)
  const again = parseNote(serializeNote(blocks))
  assert.deepEqual(again, blocks)
}