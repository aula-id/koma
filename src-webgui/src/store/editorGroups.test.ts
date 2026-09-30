import assert from 'node:assert/strict'
import {
  DEFAULT_GROUP,
  DEFAULT_TREE,
  asLayoutNode,
  dropZoneFor,
  findSplit,
  gridLayout,
  gridLayoutFromTree,
  insertGroup,
  isTabVisible,
  leafIds,
  neighbourInGroup,
  normalizeGroups,
  parentSplitOf,
  reorderTab,
  resizeGroups,
  resizeSplit,
  setSplitDir,
  toggleSplitDir,
  type EditorLayoutNode,
  type GroupLayout,
} from './editorGroups.ts'

const layout = (partial: Partial<GroupLayout> = {}): GroupLayout => ({
  tabs: [{ id: 'chat' }, { id: 'a' }, { id: 'b' }],
  activeTabId: 'b',
  groups: [DEFAULT_GROUP],
  tabGroup: { chat: DEFAULT_GROUP, a: DEFAULT_GROUP, b: DEFAULT_GROUP },
  groupActive: { [DEFAULT_GROUP]: 'b' },
  activeGroupId: DEFAULT_GROUP,
  splitDir: 'row',
  groupSizes: { [DEFAULT_GROUP]: 1 },
  splitTree: DEFAULT_TREE,
  groupSplitDir: {},
  ...partial,
})

const L2: EditorLayoutNode = {
  type: 'split',
  id: 's0',
  dir: 'row',
  aSize: 1,
  bSize: 1,
  a: {
    type: 'split',
    id: 's1',
    dir: 'col',
    aSize: 1,
    bSize: 1,
    a: { type: 'leaf', id: 'g0' },
    b: { type: 'leaf', id: 'g1' },
  },
  b: { type: 'leaf', id: 'g2' },
}

{
  const ui = layout()
  assert.equal(normalizeGroups(ui), ui)
}

{
  const ui = layout({
    tabs: [{ id: 'chat' }, { id: 'a' }, { id: 'new' }],
    activeTabId: 'new',
    tabGroup: { chat: 'g0', a: 'g0' },
    groupActive: { g0: 'a' },
  })
  const next = normalizeGroups(ui)
  assert.equal(next.tabGroup.new, 'g0')
  assert.equal(next.groupActive.g0, 'new')
}

{
  const next = normalizeGroups(
    layout({
      groups: ['g0', 'g1'],
      tabGroup: { chat: 'g0', a: 'g0', b: 'g1' },
      groupActive: { g0: 'a', g1: 'b' },
      activeGroupId: 'g0',
      activeTabId: 'b',
      groupSizes: { g0: 1, g1: 1 },
      splitTree: {
        type: 'split',
        id: 's0',
        dir: 'row',
        aSize: 1,
        bSize: 1,
        a: { type: 'leaf', id: 'g0' },
        b: { type: 'leaf', id: 'g1' },
      },
    }),
  )
  assert.equal(next.activeGroupId, 'g1')
  assert.equal(isTabVisible(next, 'a'), true)
  assert.equal(isTabVisible(next, 'b'), true)
  assert.equal(isTabVisible(next, 'chat'), false)
}

{
  const next = normalizeGroups(
    layout({
      groups: ['g0', 'g1'],
      tabGroup: { chat: 'g0', a: 'g0', b: 'g0', ghost: 'g1' },
      groupActive: { g0: 'b', g1: 'ghost' },
      activeGroupId: 'g1',
      groupSizes: { g0: 2, g1: 1, ghost: 99 },
      splitTree: {
        type: 'split',
        id: 's0',
        dir: 'col',
        aSize: 2,
        bSize: 1,
        a: { type: 'leaf', id: 'g0' },
        b: { type: 'leaf', id: 'g1' },
      },
    }),
  )
  assert.deepEqual(next.groups, ['g0'])
  assert.equal(next.activeGroupId, 'g0')
  assert.deepEqual(next.tabGroup, { chat: 'g0', a: 'g0', b: 'g0' })
  assert.deepEqual(next.groupSizes, { g0: 1 })
  assert.equal(next.splitDir, 'row')
  assert.equal(next.splitTree?.type, 'leaf')
}

{
  const laid = gridLayout(['g0'], { g0: 1.7 }, 'col')
  assert.equal(laid.gridTemplateColumns, 'minmax(0, 1fr)')
  assert.equal(laid.gridTemplateRows, 'auto minmax(0, 1fr)')
  assert.equal(laid.cells.length, 1)
  assert.equal(laid.cells[0].grip, null)
  assert.deepEqual(laid.cells[0].bar, { gridColumn: '1', gridRow: '1' })
  assert.deepEqual(laid.cells[0].content, { gridColumn: '1', gridRow: '2' })
}

{
  const ui = layout({
    tabs: [{ id: 'chat' }, { id: 'a' }, { id: 'b' }, { id: 'c' }],
    groups: ['g0', 'g1'],
    tabGroup: { chat: 'g0', a: 'g0', b: 'g1', c: 'g1' },
    groupActive: { g0: 'a', g1: 'c' },
    groupSizes: { g0: 1, g1: 1 },
  })
  assert.equal(neighbourInGroup(ui, 'c'), 'b')
  assert.equal(neighbourInGroup(ui, 'b'), 'c')
}

{
  const tabs = [{ id: 'chat' }, { id: 'a' }, { id: 'b' }, { id: 'c' }]
  assert.deepEqual(reorderTab(tabs, 'c', 'a').map((t) => t.id), ['chat', 'c', 'a', 'b'])
  assert.deepEqual(reorderTab(tabs, 'chat', 'b').map((t) => t.id), ['chat', 'a', 'b', 'c'])
}

{
  const first = insertGroup(layout(), 'g0', 'after', 'row')
  assert.ok(first)
  assert.deepEqual(first.groups, ['g0', 'g1'])
  assert.equal(first.splitTree.type, 'split')
  if (first.splitTree.type === 'split') {
    assert.equal(first.splitTree.dir, 'row')
    assert.equal(first.splitTree.b.type, 'leaf')
    if (first.splitTree.b.type === 'leaf') assert.equal(first.splitTree.b.id, 'g1')
  }
}

{
  const two = insertGroup(layout(), 'g0', 'after', 'row')
  assert.ok(two)
  const nested = insertGroup(
    layout({
      groups: two.groups,
      splitTree: two.splitTree,
      tabGroup: { chat: 'g0', a: 'g0', b: 'g1' },
      groupActive: { g0: 'a', g1: 'b' },
      groupSizes: { g0: 1, g1: 1 },
    }),
    'g0',
    'after',
    'col',
  )
  assert.ok(nested)
  assert.deepEqual(nested.groups, ['g0', 'g2', 'g1'])
  assert.equal(parentSplitOf(nested.splitTree, 'g0')?.dir, 'col')
  assert.equal(parentSplitOf(nested.splitTree, 'g1')?.dir, 'row')
}

{
  const unsplit = layout()
  assert.equal(toggleSplitDir(unsplit), null)
  assert.equal(setSplitDir(unsplit, 'col'), null)

  const split = layout({
    groups: ['g0', 'g1'],
    tabGroup: { chat: 'g0', a: 'g0', b: 'g1' },
    groupActive: { g0: 'a', g1: 'b' },
    groupSizes: { g0: 1, g1: 1 },
    splitDir: 'row',
    splitTree: {
      type: 'split',
      id: 's0',
      dir: 'row',
      aSize: 1,
      bSize: 1,
      a: { type: 'leaf', id: 'g0' },
      b: { type: 'leaf', id: 'g1' },
    },
  })
  const flipped = toggleSplitDir(split)
  assert.ok(flipped?.splitTree && flipped.splitTree.type === 'split')
  assert.equal(flipped.splitTree.dir, 'col')
  const set = setSplitDir(split, 'col')
  assert.ok(set?.splitTree && set.splitTree.type === 'split')
  assert.equal(set.splitTree.dir, 'col')
  assert.equal(setSplitDir(split, 'row'), null)
}

{
  const nested = layout({
    tabs: [{ id: 'chat' }, { id: 'a' }, { id: 'b' }, { id: 'c' }],
    groups: ['g0', 'g1', 'g2'],
    tabGroup: { chat: 'g0', a: 'g0', b: 'g1', c: 'g2' },
    groupActive: { g0: 'a', g1: 'b', g2: 'c' },
    groupSizes: { g0: 1, g1: 1, g2: 1 },
    splitTree: L2,
    activeGroupId: 'g0',
  })
  const flipped = toggleSplitDir(nested, 'g0')
  assert.ok(flipped?.splitTree && flipped.splitTree.type === 'split')
  assert.equal(flipped.splitTree.dir, 'row')
  if (flipped.splitTree.a.type === 'split') assert.equal(flipped.splitTree.a.dir, 'row')
}

{
  const next = normalizeGroups(
    layout({
      tabs: [{ id: 'chat' }, { id: 'a' }, { id: 'b' }],
      groups: ['g0', 'g1'],
      tabGroup: { chat: 'g0', a: 'g0', b: 'g1' },
      groupActive: { g0: 'a', g1: 'b' },
      groupSizes: { g0: 2, g1: 1 },
      splitDir: 'col',
      splitTree: undefined,
    }),
  )
  assert.equal(next.splitTree?.type, 'split')
  if (next.splitTree?.type === 'split') {
    assert.equal(next.splitTree.dir, 'col')
    assert.equal(next.splitTree.aSize, 2)
    assert.deepEqual(leafIds(next.splitTree), ['g0', 'g1'])
  }
}

{
  assert.equal(dropZoneFor(5, 50, 100, 100), 'left')
  assert.equal(dropZoneFor(95, 50, 100, 100), 'right')
  assert.equal(dropZoneFor(50, 5, 100, 100), 'top')
  assert.equal(dropZoneFor(50, 95, 100, 100), 'bottom')
  assert.equal(dropZoneFor(50, 50, 100, 100), 'center')
  assert.equal(dropZoneFor(5, 50, 100, 100, { allowEdges: false }), 'center')
  assert.equal(dropZoneFor(95, 5, 100, 100, { allowEdges: false }), 'center')
}

{
  const sizes = resizeGroups(['g0', 'g1'], { g0: 1, g1: 1 }, 0, 100, 600)
  assert.ok(sizes.g0 > 1)
  assert.ok(sizes.g1 < 1)

  const base = { g0: 1, g1: 1 }
  const clamped = resizeGroups(['g0', 'g1'], base, 0, -10_000, 600)
  assert.ok(clamped.g0 > 0)
  assert.ok(clamped.g1 < 2)
  assert.equal(resizeGroups(['g0', 'g1'], clamped, 0, -10_000, 600), clamped)

  const tree: EditorLayoutNode = {
    type: 'split',
    id: 's0',
    dir: 'col',
    aSize: 1,
    bSize: 1,
    a: { type: 'leaf', id: 'g0' },
    b: { type: 'leaf', id: 'g1' },
  }
  const resized = resizeSplit(tree, 's0', 80, 400)
  assert.ok(resized.type === 'split' && resized.aSize > 1)
  const splitClamped = resizeSplit(tree, 's0', -10_000, 400)
  assert.equal(resizeSplit(splitClamped, 's0', -10_000, 400), splitClamped)
}

{
  const row = gridLayout(['g0', 'g1'], { g0: 2, g1: 1 }, 'row')
  assert.equal(row.cells[0].bar.gridColumn, '1 / 2')
  assert.equal(row.cells[0].content.gridRow, '2 / 3')
  assert.equal(row.cells[0].grip?.gridColumn, '2')
  assert.equal(row.cells[1].content.gridColumn, '3 / 4')

  const col = gridLayout(['g0', 'g1'], { g0: 1, g1: 1 }, 'col')
  assert.equal(col.cells[0].bar.gridRow, '1')
  assert.equal(col.cells[0].content.gridRow, '2 / 3')
  assert.equal(col.cells[0].grip?.gridRow, '3')
  assert.equal(col.cells[1].bar.gridRow, '4')
}

{
  const laid = gridLayoutFromTree(L2)
  assert.ok(laid.cells.length === 3)
  assert.ok(laid.grips.length === 2)
  const byId = Object.fromEntries(laid.cells.map((c) => [c.id, c]))
  assert.equal(byId.g0.bar.gridColumn, '1 / 2')
  assert.equal(byId.g2.bar.gridColumn, '3 / 4')
  assert.equal(byId.g2.content.gridRow, '2 / 6')
  assert.equal(byId.g1.bar.gridRow, '4')
  const vGrip = laid.grips.find((g) => g.dir === 'row')
  const hGrip = laid.grips.find((g) => g.dir === 'col')
  assert.equal(vGrip?.cell.gridColumn, '2')
  assert.equal(hGrip?.cell.gridRow, '3')
  assert.equal(hGrip?.cell.gridColumn, '1 / 2')
}

{
  assert.deepEqual(asLayoutNode(null), { type: 'leaf', id: DEFAULT_GROUP })
  assert.deepEqual(asLayoutNode(undefined), { type: 'leaf', id: DEFAULT_GROUP })
  assert.deepEqual(leafIds(null), [DEFAULT_GROUP])
  assert.equal(parentSplitOf(null, 'g0'), null)
  assert.equal(findSplit(undefined, 's0'), null)
  const broken = asLayoutNode({ type: 'split', id: 's0', dir: 'row', a: { type: 'leaf', id: 'g3' } })
  assert.deepEqual(broken, { type: 'leaf', id: 'g3' })
  const next = normalizeGroups(
    layout({
      splitTree: { type: 'split', id: 's0' } as never,
      tabGroup: { chat: 'g0', a: 'g0', b: 'g0' },
    }),
  )
  assert.equal(next.splitTree?.type, 'leaf')
  assert.doesNotThrow(() => gridLayoutFromTree(null))
}

console.log('editorGroups.test.ts: all assertions passed')
