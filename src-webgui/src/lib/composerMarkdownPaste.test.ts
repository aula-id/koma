import assert from 'node:assert/strict'
import { loneHttpUrl, looksLikeComposerMarkdown } from './composerMarkdownPaste.ts'

assert.equal(looksLikeComposerMarkdown('plain hello'), false)
assert.equal(looksLikeComposerMarkdown('**bold**'), true)
assert.equal(looksLikeComposerMarkdown('- [ ] todo\n- [x] done'), true)
assert.equal(looksLikeComposerMarkdown('| a | b |\n|---|---|'), true)
assert.equal(loneHttpUrl('https://example.com/a'), 'https://example.com/a')
assert.equal(loneHttpUrl('not a url'), null)

console.log('composerMarkdownPaste.test.ts ok')
