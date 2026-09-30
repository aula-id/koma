import assert from 'node:assert/strict'
import { codeSpan, escapeMarkdownText, wrapMarks } from './lexicalMarkdown.ts'

assert.equal(escapeMarkdownText('a * b ` c'), 'a \\* b \\` c')
assert.equal(wrapMarks('bold', { bold: true, italic: false, strike: false, code: false }), '**bold**')
assert.equal(wrapMarks('slant', { bold: false, italic: true, strike: false, code: false }), '*slant*')
assert.equal(wrapMarks('both', { bold: true, italic: true, strike: false, code: false }), '***both***')
assert.equal(wrapMarks('code', { bold: false, italic: false, strike: false, code: true }), '`code`')
assert.equal(wrapMarks('a`b', { bold: true, italic: false, strike: false, code: true }), '**``a`b``**')
assert.equal(codeSpan('a`b'), '``a`b``')

console.log('lexicalMarkdown.test.ts ok')
