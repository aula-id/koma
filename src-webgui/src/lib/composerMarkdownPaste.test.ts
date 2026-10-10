import assert from 'node:assert/strict'
import { CHECK_LIST, UNORDERED_LIST } from '@lexical/markdown'
import { composerTextPasteAction, loneHttpUrl, looksLikeComposerMarkdown, splitTaskListMarker } from './composerMarkdownPaste.ts'

assert.equal(looksLikeComposerMarkdown('plain hello'), false)
assert.equal(looksLikeComposerMarkdown('**bold**'), true)
assert.equal(looksLikeComposerMarkdown('- [ ] todo\n- [x] done'), true)
assert.equal(looksLikeComposerMarkdown('| a | b |\n|---|---|'), true)

assert.equal(composerTextPasteAction('hello'), 'inline')
assert.equal(composerTextPasteAction('**bold**'), 'markdown')
assert.equal(composerTextPasteAction('1. short item'), 'markdown')
assert.equal(composerTextPasteAction('1. a\n2. b'), 'collapse')
assert.equal(composerTextPasteAction('`code`\nand another line'), 'collapse')
assert.equal(composerTextPasteAction(`${'x'.repeat(150)}`), 'collapse')
assert.equal(composerTextPasteAction(`1. ${'error log with `backticks` '.repeat(8)}`), 'collapse')
assert.equal(loneHttpUrl('https://example.com/a'), 'https://example.com/a')
assert.equal(loneHttpUrl('not a url'), null)

assert.deepEqual(splitTaskListMarker('[x] adasd'), { checked: true, rest: 'adasd' })
assert.deepEqual(splitTaskListMarker('[ ] adadasd'), { checked: false, rest: 'adadasd' })
assert.equal(splitTaskListMarker('adadadada'), null)
assert.equal(CHECK_LIST.regExp.test('- [x] adasd'), true)
assert.equal(UNORDERED_LIST.regExp.test('- [x] adasd'), true)

console.log('composerMarkdownPaste.test.ts ok')
