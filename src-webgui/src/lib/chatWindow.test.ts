import assert from 'node:assert/strict'
import {
  CHAT_TURNS_DEFAULT,
  countTurns,
  isHistoryPrepend,
  isTurnStart,
  nextRenderFrom,
  pageFromTurn,
  visibleFromTurn,
  type TurnRow,
} from './chatWindow'

const prompt = (idx: number): TurnRow => ({ role: 'user', idx })
const reply = (idx: number): TurnRow => ({ role: 'assistant', idx })
const shell = (idx: number): TurnRow => ({ role: 'user', kind: 'shell', idx })
const nudge = (idx: number): TurnRow => ({ role: 'user', kind: 'bashNudge', idx })

function transcript(turns: number, lead = 0): TurnRow[] {
  const rows: TurnRow[] = []
  for (let i = 0; i < lead; i++) rows.push(reply(i))
  for (let t = 0; t < turns; t++) {
    const base = lead + t * 3
    rows.push(prompt(base), reply(base + 1), shell(base + 2))
  }
  return rows
}

const few = transcript(4)
assert.equal(visibleFromTurn(few, 30), 0, 'fewer turns than requested mounts everything')
assert.equal(countTurns(few), 4)
assert.equal(isTurnStart(shell(1)), false)
assert.equal(isTurnStart(nudge(2)), false)

const exact = transcript(CHAT_TURNS_DEFAULT, 2)
assert.equal(visibleFromTurn(exact, 30), 0, 'exactly 30 turns keeps the rows before the first prompt')
assert.ok(exact[0].role === 'assistant')

const more = transcript(31)
const from30 = visibleFromTurn(more, 30)
assert.equal(more[from30].role, 'user')
assert.equal(countTurns(more.slice(from30)), 30, 'exactly the last 30 prompts')
assert.equal(countTurns(more.slice(0, from30)), 1)
assert.ok(more.slice(from30).some((row) => row.kind === 'shell'), 'shell stays inside the turn')

const eighty = transcript(80)
const tail = visibleFromTurn(eighty, 30)
const older = pageFromTurn(eighty, tail, 30)
assert.equal(countTurns(eighty.slice(tail)), 30)
assert.equal(countTurns(eighty.slice(older)), 60, 'one setting-sized step earlier')
assert.ok(older < tail)

const anchor = eighty[tail]
const prepended = [prompt(-3), reply(-2), reply(-1), ...eighty]
assert.equal(
  nextRenderFrom(eighty, prepended, tail, false, 30, false),
  prepended.findIndex((row) => row.idx === anchor.idx),
  'reading back keeps the same row when older rows arrive',
)
assert.equal(nextRenderFrom(eighty, prepended, tail, true, 30, false), visibleFromTurn(prepended, 30))
assert.equal(
  nextRenderFrom(eighty, prepended, tail, false, 30, false, 30),
  visibleFromTurn(prepended, 60),
  'a history page requested while reading includes the new rows',
)
const appended = [...eighty, prompt(10_000), reply(10_001)]
assert.equal(
  nextRenderFrom(eighty, appended, tail, false, 30, false, 30),
  appended.findIndex((row) => row.idx === anchor.idx),
  'a new reply while reading does not drop the open page',
)
assert.equal(isHistoryPrepend(eighty, prepended), true)
assert.equal(isHistoryPrepend(eighty, appended), false)
const bothEnds = [prompt(-1), ...eighty, prompt(10_000)]
assert.equal(isHistoryPrepend(eighty, bothEnds), false)
assert.equal(
  nextRenderFrom(eighty, bothEnds, tail, false, 30, false, 30),
  bothEnds.findIndex((row) => row.idx === anchor.idx),
  'a snapshot that grows both ends keeps the anchor',
)

console.log('chatWindow.test ok')
