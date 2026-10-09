import assert from 'node:assert/strict'
import { activation, canEnable, initialSearchSettings, searchChip, searchSettingsReducer as reduce } from './web-search.ts'

let s = reduce(initialSearchSettings, { type: 'load', seq: 1 })
s = reduce(s, { type: 'reply', reply: { req_seq: 1, status: { provider: 'built_in', saved_keys: [] }, error: null } })
assert.equal(activation(s.status!, 'built_in', ''), null)
assert.equal(canEnable(s.status!, 'exa', '  '), false)
s = reduce(s, { type: 'edit', provider: 'exa', key: ' secret ' })
assert.equal(s.status?.provider, 'built_in') // Typing only changes the draft.
assert.deepEqual(activation(s.status!, 'exa', s.drafts.exa!), { provider: 'exa', key: 'secret' })
s = reduce(s, { type: 'begin', provider: 'exa', seq: 2 })
assert.equal(s.status?.provider, 'built_in') // No optimistic activation.
assert.equal(reduce(s, { type: 'edit', provider: 'exa', key: 'other' }), s)
assert.equal(reduce(s, { type: 'begin', provider: 'tavily', seq: 3 }), s)
assert.equal(reduce(s, { type: 'reply', reply: { req_seq: 99, status: { provider: 'exa', saved_keys: ['exa'] }, error: null } }), s)
s = reduce(s, { type: 'reply', reply: { req_seq: 2, status: { provider: 'built_in', saved_keys: [] }, error: 'Could not save' } })
assert.equal(s.status?.provider, 'built_in')
assert.equal(s.drafts.exa, ' secret ')
assert.equal(s.pending, null)
s = reduce(s, { type: 'begin', provider: 'exa', seq: 4 })
s = reduce(s, { type: 'reply', reply: { req_seq: 4, status: { provider: 'exa', saved_keys: ['exa'] }, error: null } })
assert.equal(s.drafts.exa, '')
assert.equal(s.status?.provider, 'exa')
assert.equal(reduce(s, { type: 'edit', provider: 'exa', key: 'replacement' }), s) // Off first.
assert.deepEqual(activation(s.status!, 'exa', ''), { provider: 'built_in', key: null })
s = reduce(s, { type: 'begin', provider: 'exa', seq: 5 })
s = reduce(s, { type: 'reply', reply: { req_seq: 5, status: { provider: 'built_in', saved_keys: ['exa'] }, error: null } })
assert.equal(canEnable(s.status!, 'exa', ''), true)
assert.deepEqual(activation(s.status!, 'exa', ''), { provider: 'exa', key: null })
s = reduce(s, { type: 'edit', provider: 'exa', key: 'replacement' })
assert.deepEqual(activation(s.status!, 'exa', s.drafts.exa!), { provider: 'exa', key: 'replacement' })
assert.deepEqual(initialSearchSettings.drafts, {}) // New settings mount discards drafts.
assert.equal(searchChip(undefined), 'default')
assert.equal(searchChip('built_in'), 'default')
assert.equal(searchChip('tavily'), 'tavily')
assert.equal(searchChip('firecrawl'), 'firecrawl')
assert.equal(searchChip('exa'), 'exa')
console.log('Web search GUI state tests passed')
