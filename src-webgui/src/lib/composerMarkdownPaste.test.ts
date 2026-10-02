import assert from 'node:assert/strict'
import { CHECK_LIST, UNORDERED_LIST } from '@lexical/markdown'
import { loneHttpUrl, looksLikeComposerMarkdown, splitTaskListMarker } from './composerMarkdownPaste.ts'

assert.equal(looksLikeComposerMarkdown('plain hello'), false)
assert.equal(looksLikeComposerMarkdown('**bold**'), true)
assert.equal(looksLikeComposerMarkdown('- [ ] todo\n- [x] done'), true)
assert.equal(looksLikeComposerMarkdown('| a | b |\n|---|---|'), true)
assert.equal(loneHttpUrl('https://example.com/a'), 'https://example.com/a')
assert.equal(loneHttpUrl('not a url'), null)

assert.deepEqual(splitTaskListMarker('[x] adasd'), { checked: true, rest: 'adasd' })
assert.deepEqual(splitTaskListMarker('[ ] adadasd'), { checked: false, rest: 'adadasd' })
assert.equal(splitTaskListMarker('adadadada'), null)
assert.equal(CHECK_LIST.regExp.test('- [x] adasd'), true)
assert.equal(UNORDERED_LIST.regExp.test('- [x] adasd'), true)

console.log('composerMarkdownPaste.test.ts ok')
