import assert from 'node:assert/strict'
import { applyWorkspaceTextEdits } from './workspace-edit-text'
const edit = (start: number, end: number, newText: string) => ({ range: { start: { line: 0, character: start }, end: { line: 0, character: end } }, newText })
assert.equal(applyWorkspaceTextEdits('a😀b', [edit(1, 3, 'x')]), 'axb')
assert.equal(applyWorkspaceTextEdits('abc', [edit(1, 1, 'x'), edit(1, 1, 'y')]), 'axybc')
assert.equal(applyWorkspaceTextEdits('abc', [edit(0, 1, 'X'), edit(2, 3, 'Z')]), 'XbZ')
assert.throws(() => applyWorkspaceTextEdits('abc', [edit(0, 2, 'X'), edit(1, 3, 'Z')]), /overlap/)
assert.throws(() => applyWorkspaceTextEdits('abc', [edit(0, 4, 'X')]), /outside/)
assert.throws(() => applyWorkspaceTextEdits('abc', [edit(2, 1, 'X')]), /overlap/)
