import assert from 'node:assert/strict'
import { codingAskInChatLabel, codingAskInChatPaste, codingRangeToken } from './codingRef.ts'

{
  assert.equal(codingAskInChatLabel('src-agent/src/app/cascade.rs', 62, 67), 'cascade.rs:62:67')
  assert.equal(codingAskInChatLabel('src-agent/src/app/cascade.rs', 22, 22), 'cascade.rs:22')
  assert.equal(codingAskInChatLabel('cascade.rs', 1, 2), 'cascade.rs:1:2')
}

{
  const paste = codingAskInChatPaste('src-agent/src/app/cascade.rs', 62, 67, 'let mut m = 1;\n')
  assert.ok(paste)
  assert.equal(paste.label, 'cascade.rs:62:67')
  assert.equal(paste.path, 'src-agent/src/app/cascade.rs:62:67')
  assert.equal(paste.text, 'let mut m = 1;')
  assert.equal(codingAskInChatPaste('a.rs', 1, 1, '   \n'), null)
}

{
  const token = codingRangeToken('/ws', 'src/app.rs', ['/ws'], 10, 12)
  assert.equal(token, '@src/app.rs:10-12 ')
}

console.log('codingRef.test.ts ok')
