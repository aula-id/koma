import assert from 'node:assert/strict'
import {
  composerPreviewMarkdown,
  findFileRefRanges,
  listComposerTokens,
  moveComposerToken,
} from './composerSegments.ts'

{
  const text = 'hello @src/foo.ts and [Image #2] end'
  const tokens = new Set(['@src/foo.ts'])
  const listed = listComposerTokens(text, tokens)
  assert.equal(listed.length, 2)
  assert.equal(listed[0]?.label, '@src/foo.ts')
  assert.equal(listed[1]?.label, '[Image #2]')
  const moved = moveComposerToken(text, listed[1].start, listed[1].end, 0)
  assert.equal(moved.text.startsWith('[Image #2]hello'), true)
}

{
  const preview = composerPreviewMarkdown('see [Pasted Text #1] ok')
  assert.equal(preview, 'see `[Pasted Text #1]` ok')
}

{
  const ranges = findFileRefRanges('x @[2]/tmp/a y', new Set())
  assert.equal(ranges.length, 1)
  assert.equal(ranges[0]?.[0], 2)
}

console.log('composerSegments.test.ts ok')
