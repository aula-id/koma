import assert from 'node:assert/strict'
import {
  DEFAULT_GROUP,
  dropZoneFor,
  gridLayout,
  insertGroup,
  isTabVisible,
  neighbourInGroup,
  normalizeGroups,
  reorderTab,
  resizeGroups,
  setSplitDir,
  toggleSplitDir,
  type GroupLayout,
} from './editorGroups'

const layout = (partial: Partial<GroupLayout> = {}): GroupLayout => ({
  tabs: [{ id: 'chat' }, { id: 'a' }, { id: 'b' }],
  activeTabId: 'b',
  groups: [DEFAULT_GROUP],
  tabGroup: { chat: DEFAULT_GROUP, a: DEFAULT_GROUP, b: DEFAULT_GROUP },
  groupActive: { [DEFAULT_GROUP]: 'b' },
  activeGroupId: DEFAULT_GROUP,
  splitDir: 'row',
  groupSizes: { [DEFAULT_GROUP]: 1 },
  ...partial,
})

// Identity is stable when every invariant already holds.
{
  const ui = layout()
  assert.equal(normalizeGroups(ui), ui)
}

// Newly opened, unstamped tabs join the focused group and become its active tab.
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

// activeTabId drives focus into the group that owns the activated tab.
{
  const next = normalizeGroups(
    layout({
      groups: ['g0', 'g1'],
      tabGroup: { chat: 'g0', a: 'g0', b: 'g1' },
      groupActive: { g0: 'a', g1: 'b' },
      activeGroupId: 'g0',
      activeTabId: 'b',
      groupSizes: { g0: 1, g1: 1 },
    }),
  )
  assert.equal(next.activeGroupId, 'g1')
  assert.equal(isTabVisible(next, 'a'), true)
  assert.equal(isTabVisible(next, 'b'), true)
  assert.equal(isTabVisible(next, 'chat'), false)
}

// Empty groups collapse and stale maps are removed.
{
  const next = normalizeGroups(
    layout({
      groups: ['g0', 'g1'],
      tabGroup: { chat: 'g0', a: 'g0', b: 'g0', ghost: 'g1' },
      groupActive: { g0: 'b', g1: 'ghost' },
      activeGroupId: 'g1',
      groupSizes: { g0: 2, g1: 1, ghost: 99 },
    }),
  )
  assert.deepEqual(next.groups, ['g0'])
  assert.equal(next.activeGroupId, 'g0')
  assert.deepEqual(next.tabGroup, { chat: 'g0', a: 'g0', b: 'g0' })
  // Collapse resets the survivor to unit weight (not the pre-split 2).
  assert.deepEqual(next.groupSizes, { g0: 1 })
  assert.equal(next.splitDir, 'row')
}

// Unsplit grid is always a single explicit 1fr track (no grip, ignore sizes/dir).
{
  const laid = gridLayout(['g0'], { g0: 1.7 }, 'col')
  assert.equal(laid.gridTemplateColumns, 'minmax(0, 1fr)')
  assert.equal(laid.gridTemplateRows, 'auto minmax(0, 1fr)')
  assert.equal(laid.cells.length, 1)
  assert.equal(laid.cells[0].grip, null)
  assert.deepEqual(laid.cells[0].bar, { gridColumn: '1', gridRow: '1' })
  assert.deepEqual(laid.cells[0].content, { gridColumn: '1', gridRow: '2' })
}

// Closing fallback stays within its own group.
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

// Reordering never moves the permanent first tab.
{
  const tabs = [{ id: 'chat' }, { id: 'a' }, { id: 'b' }, { id: 'c' }]
  assert.deepEqual(reorderTab(tabs, 'c', 'a').map((t) => t.id), ['chat', 'c', 'a', 'b'])
  assert.deepEqual(reorderTab(tabs, 'chat', 'b').map((t) => t.id), ['chat', 'a', 'b', 'c'])
}

// Group insertion is bounded at two panes and preserves the requested side/orientation.
{
  const first = insertGroup(layout(), 'g0', 'after', 'row')
  assert.ok(first)
  assert.deepEqual(first.groups, ['g0', 'g1'])
  assert.equal(first.splitDir, 'row')

  const atLimit = layout({
    groups: ['g0', 'g1'],
    groupSizes: { g0: 1, g1: 1 },
  })
  assert.equal(insertGroup(atLimit, 'g1', 'after', 'col'), null)
}

// Axis flip only when two panes are open.
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
  })
  assert.deepEqual(toggleSplitDir(split), { splitDir: 'col' })
  assert.deepEqual(setSplitDir(split, 'col'), { splitDir: 'col' })
  assert.equal(setSplitDir(split, 'row'), null)
}

// Edge drop zones and center move zone (edges optional once already split).
{
  assert.equal(dropZoneFor(5, 50, 100, 100), 'left')
  assert.equal(dropZoneFor(95, 50, 100, 100), 'right')
  assert.equal(dropZoneFor(50, 5, 100, 100), 'top')
  assert.equal(dropZoneFor(50, 95, 100, 100), 'bottom')
  assert.equal(dropZoneFor(50, 50, 100, 100), 'center')
  assert.equal(dropZoneFor(5, 50, 100, 100, { allowEdges: false }), 'center')
  assert.equal(dropZoneFor(95, 5, 100, 100, { allowEdges: false }), 'center')
}

// Resize trades only between adjacent groups and clamps away from zero.
// Identity is stable at the clamp edge (same object returned).
{
  const sizes = resizeGroups(['g0', 'g1'], { g0: 1, g1: 1 }, 0, 100, 600)
  assert.ok(sizes.g0 > 1)
  assert.ok(sizes.g1 < 1)

  const base = { g0: 1, g1: 1 }
  const clamped = resizeGroups(['g0', 'g1'], base, 0, -10_000, 600)
  assert.ok(clamped.g0 > 0)
  assert.ok(clamped.g1 < 2)
  // Further delta at the min clamp must not allocate a new map.
  assert.equal(resizeGroups(['g0', 'g1'], clamped, 0, -10_000, 600), clamped)
}

// Grid cells put strips above their content and grips between panes.
{
  const row = gridLayout(['g0', 'g1'], { g0: 2, g1: 1 }, 'row')
  assert.equal(row.cells[0].bar.gridColumn, '1')
  assert.equal(row.cells[0].content.gridRow, '2')
  assert.equal(row.cells[0].grip?.gridColumn, '2')
  assert.equal(row.cells[1].content.gridColumn, '3')

  const col = gridLayout(['g0', 'g1'], { g0: 1, g1: 1 }, 'col')
  assert.equal(col.cells[0].bar.gridRow, '1')
  assert.equal(col.cells[0].content.gridRow, '2')
  assert.equal(col.cells[0].grip?.gridRow, '3')
  assert.equal(col.cells[1].bar.gridRow, '4')
}

console.log('editorGroups.test.ts: all assertions passed')
