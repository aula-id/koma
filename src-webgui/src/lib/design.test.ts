import assert from 'node:assert/strict'
import {
  alignDesignNodes,
  addComponentVariant,
  addDesignToken,
  applyDesignStyle,
  applyOverrides,
  componentView,
  copyTree,
  createComponentFromFrame,
  createNode,
  deleteDesignComponent,
  deleteDesignNode,
  designChatNote,
  designChatText,
  designCoordinateText,
  canLeaveParent,
  designCanvasBox,
  designDrop,
  effectiveInstanceChild,
  designObjectSnap,
  designPath,
  designSnapScene,
  designStyle,
  designQueryNode,
  nodeBoxOrigin,
  designFenceTitle,
  splitDesignMessage,
  designFileName,
  designLayerName,
  flipDesignNode,
  moveDesignNode,
  nodeFromPen,
  orderDesignNode,
  selectDesignRect,
  setDesignLocked,
  setDesignVisible,
  snapDesign,
  stackDesign,
  vectorSvgPath,
  wrapDesignNodes,
  unwrapDesignNode,
  layerDropIndex,
  hitGroupChild,
  dropDesignToken,
  layoutDesign,
  makeInstance,
  measureTextBox,
  mergeDesignOverride,
  reorderDesignNode,
  duplicateDesignNodes,
  emptyDesign,
  findDesignNode,
  frameAtPoint,
  frameDesignView,
  hitDesign,
  designEnterScopeForLayerSelect,
  exitDesignContainer,
  resolveDesignSelectHit,
  selectDesignHit,
  validateDesignEnteredContainer,
  insertDesignNode,
  isDesignPath,
  nodeChrome,
  nodeOrigin,
  nudgeDesignNodes,
  parseDesign,
  pickVariant,
  placeDesignNode,
  queryDesign,
  resizeDesignNode,
  resetInstanceOverrides,
  resolveInstanceTree,
  resolveRef,
  selectAllDesign,
  serializeDesign,
  sharedValue,
  setDesignMode,
  setDesignTokenValue,
  setInstanceVariant,
  setVariantProps,
  updateDesignNode,
  writeComponentView,
  textStyle,
  variantKey,
} from './design.ts'
import type { DesignComponent, DesignDoc, DesignNode } from './design.ts'

assert.equal(designFileName(' login '), 'login.kdsgn')
assert.equal(designFileName('login.kdsgn'), 'login.kdsgn')
assert.equal(designFileName('login.KDSGN'), 'login.kdsgn')
assert.equal(designFileName('my.ui'), 'my.ui.kdsgn')
assert.equal(designFileName('../x'), null)
assert.equal(designFileName('a/b'), null)
assert.equal(designFileName('.kdsgn'), null)
assert.equal(designFileName(''), null)

assert.equal(isDesignPath('.koma/login.kdsgn'), true)
assert.equal(isDesignPath('.koma/nested/login.kdsgn'), false)
assert.equal(isDesignPath('.koma/auth.diag'), false)

{
  const parsed = parseDesign(serializeDesign(emptyDesign()))
  assert.equal(parsed.error, null)
  assert.deepEqual(parsed.doc, emptyDesign())
}

{
  const parsed = parseDesign('{')
  assert.equal(parsed.error, 'This file is not a design')
  assert.deepEqual(parsed.doc, emptyDesign())
}

{
  const parsed = parseDesign('{}')
  assert.equal(parsed.error, 'This file is not a design')
}

{
  const parsed = parseDesign('{"version":2}')
  assert.equal(parsed.error, 'This file is not a design')
}

function button(): DesignComponent {
  const label = createNode('text', 'label', 12, 8)
  label.text = 'Save'
  label.color = 'color.fg'
  const frame = createNode('frame', 'button-primary', 0, 0)
  frame.name = 'Button'
  frame.layout = 'row'
  frame.pad = 8
  frame.gap = 4
  frame.wMode = 'hug'
  frame.hMode = 'hug'
  frame.fill = 'color.accent'
  frame.radius = 'radius.sm'
  frame.children = [label]
  const quiet = createNode('frame', 'button-quiet', 0, 0)
  quiet.name = 'Button'
  quiet.layout = 'row'
  quiet.fill = 'none'
  quiet.children = [{ ...label, id: 'label-quiet', text: 'Save' }]
  return {
    id: 'button',
    name: 'Button',
    axes: { tone: ['primary', 'quiet'] },
    variants: [
      { props: { tone: 'primary' }, node: frame },
      { props: { tone: 'quiet' }, node: quiet },
    ],
  }
}

function sample(): DesignDoc {
  const doc = emptyDesign()
  doc.tokens = [
    { name: 'color.fg', kind: 'color', values: { light: '#111111', dark: '#eeeeee' } },
    { name: 'color.accent', kind: 'color', values: { light: '#3355ff', dark: '#8899ff' } },
    { name: 'space.2', kind: 'space', values: { light: '8', dark: '8' } },
    { name: 'radius.sm', kind: 'radius', values: { light: '6', dark: '6' } },
    { name: 'type.body', kind: 'type', values: { light: '14/regular', dark: '14/regular' } },
    { name: 'color.unused', kind: 'color', values: { light: '#abcdef', dark: '#abcdef' } },
  ]
  doc.components = [button(), { id: 'field', name: 'Field', variants: [{ props: {}, node: createNode('frame', 'field', 0, 0) }] }]
  const screen = createNode('frame', 'login', 40, 24)
  screen.name = 'Login'
  screen.layout = 'column'
  screen.gap = 12
  screen.pad = 16
  const instance: DesignNode = {
    id: 'submit',
    kind: 'instance',
    x: 0,
    y: 0,
    w: 120,
    h: 36,
    component: 'button',
    variant: { tone: 'primary' },
    text: 'Continue',
  }
  screen.children = [instance]
  doc.screens = [screen]
  return doc
}

{
  const doc = sample()
  const parsed = parseDesign(serializeDesign(doc))
  assert.equal(parsed.error, null)
  assert.deepEqual(parsed.doc, doc)
  const raw = JSON.parse(serializeDesign(doc)) as Record<string, unknown>
  assert.equal(raw.snap, undefined)
  assert.equal(raw.grid, undefined)
  assert.equal(raw.mode, undefined)
}

{
  const text = createNode('text', 't', 0, 0)
  text.fontSize = 13
  text.weight = 'regular'
  text.textAlign = 'left'
  const frame = createNode('frame', 'f', 0, 0)
  frame.align = 'start'
  frame.children = [text]
  const doc = emptyDesign()
  doc.screens = [frame]
  const parsed = parseDesign(serializeDesign(doc))
  assert.equal(parsed.error, null)
  const child = parsed.doc.screens[0].children?.[0]
  assert.ok(child)
  assert.equal(child.fontSize, undefined)
  assert.equal(child.weight, undefined)
  assert.equal(child.textAlign, undefined)
  assert.equal(parsed.doc.screens[0].align, undefined)
  assert.equal(textStyle(child).fontSize, 13)
  assert.equal(textStyle(child).align, 'left')
}

{
  const frame = createNode('frame', 'f', 0, 0)
  assert.equal(nodeChrome(frame).fill, '#ffffff')
  assert.equal(nodeChrome(frame).stroke, '')
  const text = createNode('text', 't', 0, 0)
  assert.equal(nodeChrome(text).fill, 'none')
  frame.fill = 'none'
  assert.equal(nodeChrome(frame).fill, 'none')
}

{
  const doc = sample()
  assert.equal(resolveRef(doc, 'color.fg'), '#111111')
  doc.mode = 'dark'
  assert.equal(resolveRef(doc, 'color.fg'), '#eeeeee')
  assert.equal(resolveRef(doc, '#aabbcc'), '#aabbcc')
  assert.equal(resolveRef(doc, 'missing'), '')
  assert.equal(resolveRef(doc, 'none'), 'none')
}

{
  const component = button()
  assert.equal(variantKey({ tone: 'quiet', size: 'lg' }), 'size=lg&tone=quiet')
  assert.equal(pickVariant(component, { tone: 'quiet' })?.node.id, 'button-quiet')
  assert.equal(pickVariant(component, { tone: 'missing' })?.node.id, 'button-primary')
}

{
  const variant = pickVariant(button(), { tone: 'primary' })
  assert.ok(variant)
  const painted = applyOverrides(variant.node, { text: 'Continue', fill: 'color.fg' })
  assert.equal(painted.fill, 'color.fg')
  assert.equal(painted.children?.[0].text, 'Continue')
  assert.equal(variant.node.children?.[0].text, 'Save')
}

{
  let n = 0
  const copy = copyTree(button().variants[0].node, () => `c${++n}`, 16, 8)
  assert.equal(copy.id, 'c1')
  assert.equal(copy.x, 16)
  assert.equal(copy.y, 8)
  assert.equal(copy.children?.[0].id, 'c2')
  assert.equal(copy.children?.[0].x, 12)
  assert.notEqual(copy.children?.[0].id, 'label')
}

{
  const doc = sample()
  const tokens = queryDesign(doc, { tokens: true })
  assert.ok(tokens)
  assert.equal(tokens.component, undefined)
  assert.equal(tokens.screen, undefined)
  assert.equal(tokens.tokens.length, doc.tokens.length)
  assert.equal(JSON.stringify(tokens).includes('Login'), false)
}

{
  const doc = sample()
  const slice = queryDesign(doc, { component: 'button', variant: { tone: 'primary' } })
  assert.ok(slice?.component)
  assert.equal(slice.component.variant.tone, 'primary')
  assert.equal(slice.component.tree.fill, 'color.accent')
  assert.equal(slice.screen, undefined)
  const names = slice.tokens.map((token) => token.name)
  assert.deepEqual(names, ['color.fg', 'color.accent', 'radius.sm'])
  assert.equal(JSON.stringify(slice).includes('Field'), false)
  assert.equal(JSON.stringify(slice).includes('Login'), false)
}

{
  const doc = sample()
  const slice = queryDesign(doc, { screen: 'login' })
  assert.ok(slice?.screen)
  assert.equal(slice.screen.name, 'Login')
  assert.equal(slice.screen.tree.children?.[0].kind, 'instance')
  assert.equal(slice.screen.tree.children?.[0].text, 'Continue')
  assert.deepEqual(slice.components?.map((item) => item.id), ['button'])
  const names = slice.tokens.map((token) => token.name)
  assert.ok(names.includes('color.accent'))
  assert.ok(names.includes('color.fg'))
  assert.equal(names.includes('color.unused'), false)
  assert.equal(JSON.stringify(slice).includes('Field'), false)
  assert.equal(slice.component, undefined)
}

{
  const doc = sample()
  assert.equal(queryDesign(doc, { component: 'missing' }), null)
  assert.equal(queryDesign(doc, { screen: 'missing' }), null)
}

{
  const parsed = parseDesign('{"version":1,"tokens":[{"name":"nope","kind":"color","values":{"light":"red"}}]}')
  assert.equal(parsed.error, 'This file is not a design')
}

{
  const doc = emptyDesign()
  const screen = createNode('frame', 'screen', 10, 20)
  const label = createNode('text', 'label', 8, 12)
  screen.children = [label]
  doc.screens = [screen]
  assert.equal(hitDesign(doc, 12, 22)?.id, 'screen')
  assert.equal(hitDesign(doc, 20, 36)?.id, 'label')
  assert.equal(hitDesign(doc, 0, 0), null)
  assert.deepEqual(nodeOrigin(doc, 'label'), { x: 18, y: 32 })
  const grown = resizeDesignNode({ ...label, w: 120 }, 'e', 20, 0, 8, true)
  assert.equal(grown.x, 8)
  assert.equal(grown.w, 144)
  const other = createNode('frame', 'other', 400, 0)
  const nested = placeDesignNode({ ...doc, screens: [screen, other] }, 'label', 'other', 4, 6)
  assert.equal(nodeOrigin(nested, 'label')?.x, 404)
  assert.equal(frameAtPoint(nested, 20, 36, 'label'), 'screen')
  assert.equal(placeDesignNode(nested, 'other', 'other', 0, 0).screens.length, 2)
  const removed = deleteDesignNode(nested, 'label')
  assert.equal(hitDesign(removed, 410, 10)?.id, 'other')
  const added = insertDesignNode(removed, 'screen', createNode('rect', 'box', 0, 0))
  assert.equal(hitDesign(added, 12, 22)?.id, 'box')
}

{
  const row = createNode('frame', 'row', 0, 0)
  row.layout = 'row'
  row.pad = 4
  row.gap = 8
  row.w = 200
  row.h = 40
  const a = createNode('rect', 'a', 0, 0)
  a.w = 20
  a.h = 10
  const b = createNode('rect', 'b', 0, 0)
  b.w = 30
  b.h = 12
  b.wMode = 'fill'
  const pinned = createNode('rect', 'pin', 3, 5)
  pinned.absolute = true
  pinned.w = 10
  pinned.h = 10
  row.children = [a, b, pinned]
  const laid = layoutDesign({ ...emptyDesign(), screens: [row] }).screens[0]
  assert.equal(laid.children?.[0].x, 4)
  assert.equal(laid.children?.[0].y, 4)
  assert.equal(laid.children?.[0].w, 20)
  assert.equal(laid.children?.[1].x, 32)
  assert.equal(laid.children?.[1].w, 200 - 8 - 20 - 8)
  assert.equal(laid.children?.[2].x, 3)
  assert.equal(laid.children?.[2].y, 5)
  const hugged = createNode('frame', 'hug', 0, 0)
  hugged.layout = 'column'
  hugged.wMode = 'hug'
  hugged.hMode = 'hug'
  hugged.pad = 2
  hugged.gap = 2
  const one = createNode('rect', 'one', 9, 9)
  one.w = 40
  one.h = 14
  const two = createNode('rect', 'two', 9, 9)
  two.w = 16
  two.h = 10
  hugged.children = [one, two]
  const column = layoutDesign({ ...emptyDesign(), screens: [hugged] }).screens[0]
  assert.equal(column.w, 44)
  assert.equal(column.h, 30)
  assert.equal(column.children?.[1].y, 18)
  const again = layoutDesign({ ...emptyDesign(), screens: [column] }).screens[0]
  assert.deepEqual(again, column)
  const moved = reorderDesignNode({ ...emptyDesign(), screens: [column] }, 'two', 0)
  assert.equal(moved.screens[0].children?.[0].id, 'two')
}

{
  const doc = emptyDesign()
  assert.equal(addDesignToken(doc, '1bad', 'color'), null)
  const added = addDesignToken(doc, ' color.fg ', 'color')
  assert.ok(added)
  assert.equal(added.tokens[0].name, 'color.fg')
  assert.equal(added.tokens[0].values.light, '#1a1d27')
  assert.equal(added.tokens[0].values.dark, '#c8d3f5')
  assert.equal(addDesignToken(added, 'color.fg', 'space'), null)
  assert.equal(setDesignTokenValue(added, 'color.fg', 'dark', 'nope'), null)
  const dark = setDesignTokenValue(added, 'color.fg', 'dark', '#FFFFFF')
  assert.equal(dark?.tokens[0].values.dark, '#ffffff')
  assert.equal(dark?.tokens[0].values.light, '#1a1d27')
  const switched = setDesignMode(dark!, 'dark')
  assert.equal(switched.mode, 'dark')
  assert.equal(resolveRef(switched, 'color.fg'), '#ffffff')
  assert.equal(setDesignMode(switched, 'missing'), switched)
  const parsed = parseDesign(serializeDesign(switched))
  assert.equal(parsed.error, null)
  assert.equal(parsed.doc.mode, 'dark')
  assert.equal(parsed.doc.tokens[0].values.dark, '#ffffff')
  assert.equal(dropDesignToken(switched, 'color.fg').tokens.length, 0)
  assert.equal(addDesignToken(doc, 'space.2', 'space')?.tokens[0].values.light, '8')
  assert.equal(addDesignToken(doc, 'type.body', 'type')?.tokens[0].values.dark, '13/regular')
  assert.equal(addDesignToken(doc, 'radius.sm', 'radius')?.tokens[0].values.light, '8')
}

{
  const doc = emptyDesign()
  const screen = createNode('frame', 'screen', 0, 0)
  const card = createNode('frame', 'card', 16, 24)
  card.name = 'Card'
  const label = createNode('text', 'label', 4, 4)
  label.text = 'Hi'
  card.children = [label]
  screen.children = [card]
  doc.screens = [screen]
  let n = 0
  const mint = () => `m${n++}`
  const made = createComponentFromFrame(doc, 'card', 'card-1', mint)
  assert.ok(made)
  assert.equal(made.components[0].name, 'Card')
  assert.equal(made.components[0].variants[0].node.x, 0)
  assert.equal(made.components[0].variants[0].node.children?.[0].text, 'Hi')
  assert.equal(made.screens[0].children?.[0].kind, 'instance')
  assert.equal(made.screens[0].children?.[0].component, 'card-1')
  assert.equal(made.screens[0].children?.[0].x, 16)
  assert.equal(createComponentFromFrame(made, 'missing', 'nope', mint), null)
  const added = addComponentVariant(made, 'card-1', { tone: 'quiet' }, mint)
  assert.ok(added)
  assert.deepEqual(added.components[0].axes, { tone: ['quiet'] })
  assert.equal(addComponentVariant(added, 'card-1', { tone: 'quiet' }, mint), null)
  const rootId = added.components[0].variants[0].node.id
  const propped = setVariantProps(added, 'card-1', rootId, { tone: 'primary' })
  assert.deepEqual(propped?.components[0].axes?.tone, ['primary', 'quiet'])
  const view = componentView(propped!, 'card-1')
  assert.ok(view)
  assert.equal(view.screens.length, 2)
  assert.equal(view.screens[1].x > view.screens[0].x, true)
  const childId = view.screens[0].children?.[0].id
  assert.ok(childId)
  const moved = updateDesignNode(view, childId, (node) => ({ ...node, x: 12 }))
  const written = writeComponentView(propped!, 'card-1', moved)
  assert.equal(written?.screens[0].children?.[0].kind, 'instance')
  assert.equal(written?.components[0].variants[0].node.x, 0)
  assert.equal(written?.components[0].variants[0].node.children?.[0].x, 12)
  const instance = written!.screens[0].children![0]
  const tree = resolveInstanceTree(written!, { ...instance, text: 'Go', fill: '#112233' })
  assert.equal(tree?.fill, '#112233')
  assert.equal(tree?.children?.[0].text, 'Go')
  const varied = setInstanceVariant(written!, instance.id, { tone: 'quiet' })
  assert.equal(varied?.screens[0].children?.[0].variant?.tone, 'quiet')
  const painted = updateDesignNode(varied!, instance.id, (node) => ({ ...node, text: 'Go', fill: '#112233' }))
  const reset = resetInstanceOverrides(painted, instance.id)
  assert.equal(reset.screens[0].children?.[0].text, undefined)
  assert.equal(reset.screens[0].children?.[0].fill, undefined)
  const cleared = writeComponentView(propped!, 'card-1', { ...view!, screens: [] })
  assert.equal(cleared?.components.some((component) => component.id === 'card-1'), false)
  assert.equal(cleared?.screens[0].id, 'screen')
  const kept = createComponentFromFrame(doc, 'screen', 'screen-1', mint)
  assert.equal(kept?.screens[0].kind, 'frame')
  assert.equal(kept?.components[0].id, 'screen-1')
}

{
  const doc = sample()
  const component = designChatText(doc, { component: 'button', variant: { tone: 'primary' } })
  assert.ok(component?.startsWith('```kdsgn\n'))
  assert.ok(component?.endsWith('\n```'))
  assert.equal(component?.includes('@'), false)
  assert.ok(component?.includes('color.accent'))
  assert.equal(component?.includes('color.unused'), false)
  assert.equal(component?.includes('Field'), false)
  const screen = designChatText(doc, { screen: 'login' })
  assert.ok(screen?.includes('Continue'))
  assert.ok(screen?.includes('"kind":"instance"'))
  assert.equal(designChatText(doc, { component: 'missing' }), null)
  assert.equal(designChatText(doc, { screen: 'missing' }), null)
}

{
  const doc = emptyDesign()
  const frame = createNode('frame', 'screen', 0, 0)
  frame.w = 200
  frame.h = 200
  const a = createNode('rect', 'a', 10, 20)
  a.w = 40
  a.h = 30
  const b = createNode('rect', 'b', 80, 50)
  b.w = 20
  b.h = 20
  frame.children = [a, b]
  doc.screens = [frame]
  const grouped = wrapDesignNodes(doc, ['a', 'b'], 'group', 'g')
  assert.ok(grouped)
  const group = grouped!.screens[0].children?.[0]
  assert.equal(group?.kind, 'group')
  assert.equal(group?.x, 10)
  assert.equal(group?.y, 20)
  assert.equal(group?.w, 90)
  assert.equal(group?.h, 50)
  assert.equal(group?.children?.[0].id, 'a')
  assert.equal(group?.children?.[0].x, 0)
  assert.equal(group?.children?.[1].id, 'b')
  assert.equal(group?.children?.[1].x, 70)
  assert.equal(group?.children?.[1].y, 30)
  assert.equal(wrapDesignNodes(grouped!, ['screen', 'a'], 'group', 'bad'), null)
  const front = orderDesignNode(grouped!, 'a', 'front')
  assert.equal(front.screens[0].children?.[0].children?.[1].id, 'a')
  const back = orderDesignNode(front, 'a', 'back')
  assert.equal(back.screens[0].children?.[0].children?.[0].id, 'a')
  const hidden = setDesignVisible(back, 'b', false)
  assert.equal(hitDesign(hidden, 90, 60)?.id, 'g')
  assert.equal(stackDesign(hidden, 20, 30)[0]?.id, 'a')
  const locked = setDesignLocked(back, 'a', true)
  assert.equal(hitDesign(locked, 20, 30)?.id, 'g')
  assert.equal(hitDesign(locked, 20, 30, true)?.id, 'a')
  const flipped = flipDesignNode(back, 'g', 'x')
  assert.equal(flipped.screens[0].children?.[0].flipX, true)
  assert.equal(flipDesignNode(flipped, 'g', 'x').screens[0].children?.[0].flipX, undefined)
  const moved = moveDesignNode(back, 'b', 'screen', 0)
  assert.equal(moved.screens[0].children?.[0].id, 'b')
  assert.equal(moved.screens[0].children?.[0].x, 80)
  assert.equal(moved.screens[0].children?.[1].kind, 'group')
  const picked = selectDesignRect(back, 0, 0, 200, 200)
  assert.deepEqual(picked, ['screen'])
  const inner = selectDesignRect(back, 12, 22, 30, 20)
  assert.deepEqual(inner, ['a'])
  const round = parseDesign(serializeDesign(flipped))
  assert.equal(round.error, null)
  assert.equal(round.doc.screens[0].children?.[0].kind, 'group')
  assert.equal(round.doc.screens[0].children?.[0].flipX, true)
  assert.equal(designLayerName(a), 'Rectangle')
}

{
  const doc = emptyDesign()
  const frame = createNode('frame', 'screen', 0, 0)
  const box = createNode('rect', 'box', 0, 0)
  box.w = 100
  box.h = 20
  box.rotation = 90
  frame.children = [box]
  doc.screens = [frame]
  assert.equal(hitDesign(doc, 50, 50)?.id, 'box')
  assert.equal(hitDesign(doc, 0, 10)?.id, 'screen')
  const vector = nodeFromPen('path', [
    { x: 10, y: 10, incoming: { x: 0, y: 0 }, outgoing: { x: 20, y: 0 } },
    { x: 80, y: 10, incoming: { x: -20, y: 0 }, outgoing: { x: 0, y: 0 } },
    { x: 80, y: 60, incoming: { x: 0, y: 0 }, outgoing: { x: 0, y: 0 } },
  ], true)
  assert.ok(vector)
  assert.equal(vector!.x, 10)
  assert.equal(vector!.y, 10)
  assert.equal(vector!.w, 70)
  assert.equal(vector!.h, 50)
  assert.equal(vector!.vector?.vertices[0].x, 0)
  assert.equal(vector!.vector?.regions.length, 1)
  assert.ok(vectorSvgPath(vector!.vector!).includes('Z'))
  const saved = parseDesign(serializeDesign({ ...emptyDesign(), screens: [{ ...frame, children: [vector!] }] }))
  assert.equal(saved.error, null)
  assert.equal(saved.doc.screens[0].children?.[0].kind, 'vector')
  assert.equal(saved.doc.screens[0].children?.[0].vector?.segments.length, 3)
}

{
  assert.equal(snapDesign(12.4, 8, false), 12)
  assert.equal(snapDesign(12.4, 8, true), 16)
  const parsed = parseDesign('{"version":1,"screens":[{"id":"s","kind":"frame","x":0,"y":0,"w":10,"h":10}]}')
  assert.equal(parsed.doc.snap, false)
}

{
  const rect = createNode('rect', 'r', 4, 6)
  assert.equal(rect.w, 100)
  assert.equal(rect.h, 100)
  assert.equal(rect.fill, '#d9d9d9')
  assert.equal(rect.stroke, 'none')
  assert.equal(rect.radius, undefined)
  const ellipse = createNode('ellipse', 'e', 0, 0)
  assert.equal(ellipse.w, 100)
  assert.equal(ellipse.h, 100)
  assert.equal(ellipse.fill, '#d9d9d9')
  assert.equal(ellipse.stroke, 'none')
}

{
  const text = createNode('text', 'copy', 0, 0)
  assert.equal(text.w, 200)
  assert.equal(text.h, 24)
  assert.equal(text.fontSize, 14)
  text.lineHeight = 20
  text.letterSpacing = -0.5
  const frame = createNode('frame', 'f', 0, 0)
  frame.children = [text]
  const doc = { ...emptyDesign(), screens: [frame] }
  const parsed = parseDesign(serializeDesign(doc))
  const child = parsed.doc.screens[0].children?.[0]
  assert.equal(child?.fontSize, 14)
  assert.equal(child?.lineHeight, 20)
  assert.equal(child?.letterSpacing, -0.5)
  const slice = queryDesign(parsed.doc, { screen: 'f' })
  assert.equal(slice?.screen?.tree.children?.[0].lineHeight, 20)
  assert.equal(slice?.screen?.tree.children?.[0].letterSpacing, -0.5)
  if (child) {
    child.lineHeight = 0
    child.letterSpacing = 0
  }
  const omitted = parseDesign(serializeDesign({ ...emptyDesign(), screens: [{ ...frame, children: [child!] }] }))
  assert.equal(omitted.doc.screens[0].children?.[0].lineHeight, undefined)
  assert.equal(omitted.doc.screens[0].children?.[0].letterSpacing, undefined)
  assert.equal(sharedValue([1, 1, 1]), 1)
  assert.equal(sharedValue([1, 2]), null)
  assert.equal(sharedValue<string>([]), null)
}

{
  const frame = createNode('frame', 'board', 0, 0)
  frame.w = 400
  frame.h = 400
  const left = createNode('rect', 'left', 0, 10)
  const right = createNode('rect', 'right', 40, 0)
  frame.children = [left, right]
  const doc = { ...emptyDesign(), screens: [frame] }
  const aligned = alignDesignNodes(doc, ['left', 'right'], 'horizontal', 'min')
  assert.equal(aligned.screens[0].children?.[0].x, 0)
  assert.equal(aligned.screens[0].children?.[0].y, 10)
  assert.equal(aligned.screens[0].children?.[1].x, 0)
  assert.equal(aligned.screens[0].children?.[1].y, 0)
  const centered = alignDesignNodes(doc, ['left', 'right'], 'vertical', 'center')
  assert.equal(centered.screens[0].children?.[0].y, 5)
  assert.equal(centered.screens[0].children?.[1].y, 5)
}

{
  const loose = createNode('rect', 'loose', 12, 18)
  const saved = parseDesign(serializeDesign({ ...emptyDesign(), screens: [loose] }))
  assert.equal(saved.error, null)
  assert.equal(saved.doc.screens[0].kind, 'rect')
  assert.equal(saved.doc.screens[0].x, 12)
  const frame = createNode('frame', 'board', 100, 40)
  frame.children = [createNode('rect', 'inner', 10, 20)]
  const lifted = placeDesignNode({ ...emptyDesign(), screens: [frame] }, 'inner', null, 110, 60)
  assert.equal(lifted.screens.map((screen) => screen.id).join(','), 'board,inner')
  assert.equal(lifted.screens[1].x, 110)
  assert.equal(lifted.screens[1].y, 60)
  assert.equal(lifted.screens[0].children?.some((child) => child.id === 'inner') ?? false, false)
  const fence = designChatText(saved.doc, { screen: 'loose' })
  assert.equal(designFenceTitle(fence ?? ''), 'Rectangle')
  const split = splitDesignMessage(`look\n\n${fence}`)
  assert.equal(split.prose, 'look')
  assert.equal(split.designs.length, 1)
  assert.equal(split.designs[0].title, 'Rectangle')
  const legacy = splitDesignMessage(`look\n\n${designChatNote(saved.doc, { screen: 'loose' }) ?? ''}`)
  assert.equal(legacy.prose, 'look')
  assert.equal(legacy.designs.length, 1)
  const note = designChatNote(saved.doc, { screen: 'loose' })
  assert.ok(note?.includes('Attached image is the render'))
  assert.ok(note?.includes('Rectangle 100×100 at (12, 18)'))
  assert.ok(note?.includes('```kdsgn'))
  assert.equal(designQueryNode(saved.doc, { screen: 'loose' })?.id, 'loose')
}

{
  const frame = createNode('frame', 'board', 0, 0)
  frame.w = 400
  frame.h = 300
  const rect = createNode('rect', 'r', 40, 50)
  rect.flipX = true
  const group = createNode('group', 'g', 30, 40)
  group.w = 160
  group.h = 140
  group.children = [rect]
  frame.children = [group]
  const doc = { ...emptyDesign(), screens: [frame] }
  assert.deepEqual(designPath(doc, 'r')?.map((node) => node.id), ['board', 'g', 'r'])
  assert.deepEqual(nodeBoxOrigin(doc, 'r'), { x: 70, y: 90 })
  assert.equal(nodeOrigin(doc, 'r')?.x, 170)
  assert.equal(designDrop(doc, 'r', 120, 140).kind, 'stay')
  assert.equal(designDrop(doc, 'r', 900, 900).kind, 'stay')
  const lockedFrame = setDesignLocked(doc, 'board', true)
  assert.equal(designDrop(lockedFrame, 'r', 900, 900).kind, 'stay')
  assert.equal(canLeaveParent(lockedFrame, 'r', null), false)
  assert.equal(canLeaveParent(lockedFrame, 'r', 'board'), true)
  assert.equal(designDrop(setDesignLocked(lockedFrame, 'board', false), 'r', 900, 900).kind, 'stay')
  const direct = createNode('rect', 'd', 420, 40)
  direct.w = 80
  direct.h = 40
  direct.flipX = true
  const directFrame = createNode('frame', 'board2', 0, 0)
  directFrame.w = 400
  directFrame.h = 300
  directFrame.children = [direct]
  const cleared = designDrop({ ...emptyDesign(), screens: [directFrame] }, 'd', 900, 900)
  assert.equal(cleared.kind, 'move')
  if (cleared.kind === 'move') {
    assert.equal(cleared.parentId, null)
    assert.equal(cleared.x, 420)
    assert.equal(cleared.y, 40)
  }
  const overlap = createNode('rect', 'o', 350, 20)
  overlap.w = 80
  overlap.h = 40
  directFrame.children = [overlap]
  assert.equal(designDrop({ ...emptyDesign(), screens: [directFrame] }, 'o', 900, 900).kind, 'stay')
  const outer = createNode('frame', 'outer', 0, 0)
  outer.w = 400
  outer.h = 300
  const inner = createNode('frame', 'inner', 20, 20)
  inner.w = 100
  inner.h = 80
  const kid = createNode('rect', 'kid', 140, 10)
  kid.w = 40
  kid.h = 40
  inner.children = [kid]
  outer.children = [inner]
  const nest = { ...emptyDesign(), screens: [outer] }
  const upOne = designDrop(nest, 'kid', 900, 900)
  assert.equal(upOne.kind, 'move')
  if (upOne.kind === 'move') {
    assert.equal(upOne.parentId, 'outer')
    assert.equal(upOne.x, 160)
    assert.equal(upOne.y, 30)
  }
  assert.equal(designDrop(setDesignLocked(nest, 'outer', true), 'kid', 900, 900).kind, 'stay')
  const nested = createNode('frame', 'inner', 20, 20)
  nested.w = 200
  nested.h = 160
  nested.children = [group]
  frame.children = [nested]
  const nestedDoc = { ...emptyDesign(), screens: [frame] }
  const into = designDrop(nestedDoc, 'r', 120, 140)
  assert.equal(into.kind, 'stay')
  const removed = deleteDesignComponent({ ...doc, components: [{ id: 'button', name: 'Button', variants: [{ props: {}, node: createNode('frame', 'root', 0, 0) }] }] }, 'button')
  assert.equal(removed?.components.length, 0)
  assert.equal(deleteDesignComponent(doc, 'missing'), null)
  assert.ok(designCoordinateText(doc, frame).includes('Rectangle 100×100 at (40, 50)'))
  assert.ok(designCoordinateText(doc, frame).includes('flipX'))
}

{
  const frame = createNode('frame', 'board', 0, 0)
  frame.w = 400
  frame.h = 200
  const a = createNode('rect', 'a', 10, 20)
  a.w = 40
  a.h = 30
  const b = createNode('rect', 'b', 80, 40)
  b.w = 30
  b.h = 20
  const c = createNode('rect', 'c', 140, 10)
  c.w = 20
  c.h = 20
  frame.children = [a, b, c]
  const doc = { ...emptyDesign(), screens: [frame] }
  const grouped = wrapDesignNodes(doc, ['a', 'c'], 'group', 'g')
  assert.deepEqual(grouped?.screens[0].children?.map((node) => node.id), ['b', 'g'])
  assert.equal(nodeBoxOrigin(grouped!, 'a')?.x, 10)
  assert.equal(nodeBoxOrigin(grouped!, 'a')?.y, 20)
  assert.equal(nodeBoxOrigin(grouped!, 'c')?.x, 140)
  assert.equal(hitDesign(grouped!, 20, 30)?.id, 'g')
  assert.equal(hitDesign(grouped!, 20, 30, true)?.id, 'a')
  assert.equal(hitDesign(grouped!, 90, 55)?.id, 'b')
  const shifted = updateDesignNode(grouped!, 'a', (node) => ({ ...node, x: node.x + 30 }))
  const fitted = layoutDesign(shifted)
  assert.equal(nodeBoxOrigin(fitted, 'a')?.x, 40)
  assert.equal(nodeBoxOrigin(fitted, 'a')?.y, 20)
  const flipped = flipDesignNode(grouped!, 'g', 'x')
  const opened = unwrapDesignNode(flipped, 'g')
  assert.equal(opened?.screens[0].children?.map((node) => node.id).join(','), 'b,a,c')
  assert.equal(nodeBoxOrigin(opened!, 'a')?.x, nodeBoxOrigin(flipped, 'a')?.x)
  assert.equal(nodeBoxOrigin(opened!, 'a')?.y, nodeBoxOrigin(flipped, 'a')?.y)
  assert.equal(layerDropIndex(['a', 'b', 'c'], 'b', 'before'), 2)
  assert.equal(layerDropIndex(['a', 'b', 'c'], 'b', 'after'), 1)
  const raised = moveDesignNode({ ...emptyDesign(), screens: [a, b, c] }, 'a', null, layerDropIndex(['a', 'b', 'c'], 'b', 'before') ?? 0)
  assert.deepEqual(raised.screens.map((node) => node.id), ['b', 'a', 'c'])
  const row = createNode('frame', 'row', 0, 0)
  row.w = 200
  row.h = 40
  row.layout = 'row'
  row.pad = 10
  row.justify = 'space'
  const left = createNode('rect', 'l', 0, 0)
  left.w = 40
  left.h = 20
  const right = createNode('rect', 'r', 0, 0)
  right.w = 40
  right.h = 20
  row.children = [left, right]
  const spaced = layoutDesign({ ...emptyDesign(), screens: [row] })
  assert.equal(spaced.screens[0].children?.[0].x, 10)
  assert.equal(spaced.screens[0].children?.[1].x, 150)
  assert.equal(hitGroupChild(grouped!, 'g', 20, 30)?.id, 'a')
}

{
  const frame = createNode('frame', 'board', 0, 0)
  frame.w = 200
  frame.h = 200
  const rect = createNode('rect', 'r', 20, 30)
  rect.w = 40
  rect.h = 40
  frame.children = [rect]
  const doc = { ...emptyDesign(), screens: [frame] }
  const lockedFrame = setDesignLocked(doc, 'board', true)
  assert.equal(hitDesign(lockedFrame, 30, 40)?.id, 'board')
  assert.deepEqual(selectDesignRect(lockedFrame, 0, 0, 200, 200), ['board'])
  assert.equal(hitDesign(setDesignLocked(doc, 'r', true), 30, 40)?.id, 'r')
  const group = createNode('group', 'g', 10, 10)
  group.w = 80
  group.h = 80
  const child = createNode('rect', 'a', 5, 5)
  child.w = 20
  child.h = 20
  group.children = [child]
  const grouped = { ...emptyDesign(), screens: [{ ...frame, children: [group] }] }
  assert.equal(hitDesign(setDesignLocked(grouped, 'g', true), 20, 20)?.id, 'g')
  const inner = createNode('rect', 'inner', 10, 10)
  inner.w = 30
  inner.h = 30
  const instance: DesignNode = { id: 'inst', kind: 'instance', x: 0, y: 0, w: 80, h: 60, component: 'button', children: [inner] }
  const host = createNode('frame', 'host', 0, 0)
  host.w = 200
  host.h = 200
  host.children = [instance]
  const instDoc = { ...emptyDesign(), screens: [host] }
  assert.equal(hitDesign(instDoc, 20, 20)?.id, 'inst')
  assert.equal(hitDesign(instDoc, 20, 20, true)?.id, 'inner')
  assert.equal(selectDesignHit(instDoc, 20, 20, false), 'inst')
  assert.equal(selectDesignHit(instDoc, 20, 20, true), 'inner')
  assert.equal(selectDesignHit(instDoc, 20, 20, true, 'inst'), 'inner')
  assert.equal(hitDesign(instDoc, 40, 80)?.id, 'host')
}

{
  const outer = createNode('group', 'outer', 0, 0)
  outer.w = 200
  outer.h = 200
  const inner = createNode('group', 'inner', 20, 20)
  inner.w = 80
  inner.h = 80
  const leaf = createNode('rect', 'leaf', 10, 10)
  leaf.w = 20
  leaf.h = 20
  inner.children = [leaf]
  outer.children = [inner]
  const screen = createNode('frame', 'screen', 0, 0)
  screen.w = 400
  screen.h = 400
  screen.children = [outer]
  const doc = { ...emptyDesign(), screens: [screen] }
  assert.equal(hitDesign(doc, 35, 35)?.id, 'outer')
  assert.equal(selectDesignHit(doc, 35, 35, false, 'outer'), 'inner')
  assert.equal(selectDesignHit(doc, 35, 35, false, 'inner'), 'leaf')
  assert.equal(designEnterScopeForLayerSelect(doc, 'leaf'), 'inner')
  assert.equal(designEnterScopeForLayerSelect(doc, 'inner'), 'outer')
  assert.deepEqual(exitDesignContainer(doc, 'inner'), { nextEnteredId: 'outer', selectId: 'inner' })
  assert.deepEqual(exitDesignContainer(doc, 'outer'), { nextEnteredId: 'screen', selectId: 'outer' })
  assert.equal(validateDesignEnteredContainer(doc, 'inner'), 'inner')
  assert.equal(validateDesignEnteredContainer(doc, 'missing'), null)
  assert.deepEqual(resolveDesignSelectHit(doc, 35, 35, 'inner'), { kind: 'hit', id: 'leaf' })
  assert.deepEqual(resolveDesignSelectHit(doc, 25, 25, 'inner'), { kind: 'clear' })
  assert.deepEqual(selectDesignRect(doc, 0, 0, 50, 50, 'outer'), ['inner'])
}

{
  const board = createNode('frame', 'board', 0, 0)
  const a = createNode('rect', 'a', 10, 20)
  const b = createNode('rect', 'b', 40, 20)
  const c = createNode('rect', 'c', 80, 20)
  board.children = [a, b, c]
  const doc = { ...emptyDesign(), screens: [board] }
  const nudged = nudgeDesignNodes(doc, ['b'], 3, -2)
  assert.equal(findDesignNode(nudged, 'b')?.x, 43)
  assert.equal(findDesignNode(nudged, 'b')?.y, 18)
  assert.equal(findDesignNode(nudged, 'a')?.x, 10)
  const locked = setDesignLocked(doc, 'b', true)
  assert.equal(findDesignNode(nudgeDesignNodes(locked, ['b'], 5, 0), 'b')?.x, 40)
  board.layout = 'row'
  const row = { ...emptyDesign(), screens: [board] }
  assert.deepEqual(nudgeDesignNodes(row, ['b'], 1, 0).screens[0]?.children?.map((node) => node.id), ['a', 'c', 'b'])
  assert.deepEqual(nudgeDesignNodes(row, ['b'], 0, -1).screens[0]?.children?.map((node) => node.id), ['a', 'b', 'c'])
  assert.deepEqual(nudgeDesignNodes(row, ['a', 'b'], 1, 0).screens[0]?.children?.map((node) => node.id), ['c', 'a', 'b'])
  let seq = 0
  const copied = duplicateDesignNodes({ ...emptyDesign(), screens: [{ ...board, layout: undefined, children: [a, b, c] }] }, ['a', 'c'], 10, 8, () => `copy${++seq}`)
  assert.ok(copied)
  assert.deepEqual(copied?.ids, ['copy2', 'copy1'])
  assert.deepEqual(copied?.doc.screens[0]?.children?.map((node) => node.id), ['a', 'copy2', 'b', 'c', 'copy1'])
  assert.equal(findDesignNode(copied!.doc, 'copy2')?.x, 20)
  assert.equal(findDesignNode(copied!.doc, 'copy2')?.y, 28)
  const lockedCopy = duplicateDesignNodes(setDesignLocked(doc, 'a', true), ['a'], 0, 0, () => 'copy')
  assert.equal(findDesignNode(lockedCopy!.doc, 'copy')?.locked, undefined)
  const group = createNode('group', 'g', 0, 0)
  group.children = [a]
  let nestedSeq = 0
  const nested = duplicateDesignNodes({ ...emptyDesign(), screens: [group] }, ['g', 'a'], 4, 4, () => `nest${++nestedSeq}`)
  assert.deepEqual(nested?.ids, ['nest1'])
  assert.notEqual(findDesignNode(nested!.doc, 'nest1')?.children?.[0]?.id, 'a')
  assert.deepEqual(selectAllDesign(doc, []), ['board'])
  assert.deepEqual(selectAllDesign(doc, ['b']), ['a', 'b', 'c'])
  const other = createNode('frame', 'other', 400, 0)
  assert.deepEqual(selectAllDesign({ ...doc, screens: [board, other] }, ['b', 'other']), ['board', 'other'])
  const styled = createNode('text', 'label', 0, 0)
  styled.fill = '#ff0000'
  styled.fontSize = 20
  styled.color = '#111111'
  const plain = createNode('rect', 'plain', 0, 0)
  const text = createNode('text', 'word', 0, 0)
  const style = designStyle(styled)
  const painted = applyDesignStyle({ ...emptyDesign(), screens: [plain, text] }, ['plain', 'word'], style)
  assert.equal(findDesignNode(painted, 'plain')?.fill, '#ff0000')
  assert.equal(findDesignNode(painted, 'plain')?.fontSize, undefined)
  assert.equal(findDesignNode(painted, 'word')?.fontSize, 20)
  assert.equal(findDesignNode(painted, 'word')?.color, '#111111')
  const held = applyDesignStyle(setDesignLocked({ ...emptyDesign(), screens: [plain] }, 'plain', true), ['plain'], style)
  assert.equal(findDesignNode(held, 'plain')?.fill, '#d9d9d9')
  const left = createNode('rect', 'left', 0, 0)
  left.w = 100
  left.h = 100
  const right = createNode('rect', 'right', 200, 0)
  right.w = 100
  right.h = 100
  const snapDoc = { ...emptyDesign(), screens: [left, right] }
  const scene = designSnapScene(snapDoc, ['right'])
  assert.ok(scene)
  const snapped = designObjectSnap(scene!.moving, scene!.targets, -96, 0, 5)
  assert.equal(snapped.snappedX, true)
  assert.equal(snapped.dx, -100)
  assert.equal(snapped.guides[0]?.axis, 'x')
  assert.equal(snapped.guides[0]?.at, 100)
  const apart = designObjectSnap(scene!.moving, scene!.targets, -40, 0, 5)
  assert.equal(apart.snappedX, false)
  assert.equal(apart.dx, -40)
  const gap = designObjectSnap(scene!.moving, scene!.targets, 0, 12, 5)
  assert.equal(gap.measures.find((item) => item.axis === 'x')?.label, '100')
  const frame = createNode('frame', 'frame', 0, 0)
  frame.w = 400
  frame.h = 300
  const child = createNode('rect', 'child', 10, 10)
  frame.children = [child]
  const inside = designSnapScene({ ...emptyDesign(), screens: [frame] }, ['child'])
  const toEdge = designObjectSnap(inside!.moving, inside!.targets, -6, 0, 5)
  assert.equal(toEdge.dx, -10)
  assert.equal(designCanvasBox(snapDoc, 'right')?.x, 200)
  const framed = frameDesignView([{ x: 0, y: 0, w: 100, h: 100 }], 216, 216, 0.25, 64)
  assert.equal(framed?.zoom, 1.52)
  assert.equal(framed?.panX, 16)
  assert.equal(framed?.panY, 16)
}

{
  const text = createNode('text', 'hi', 0, 0)
  text.text = 'Hi'
  text.textHug = 'width'
  const hugged = layoutDesign({ ...emptyDesign(), screens: [text] }).screens[0]
  const box = measureTextBox(text)
  assert.equal(hugged.w, box.w)
  assert.equal(hugged.h, box.h)
  const block = createNode('text', 'block', 0, 0)
  block.text = 'HelloHello'
  block.w = 20
  block.textHug = 'height'
  const wrapped = layoutDesign({ ...emptyDesign(), screens: [block] }).screens[0]
  assert.equal(wrapped.w, 20)
  assert.equal(wrapped.h > box.h, true)
  const family = createNode('text', 'face', 0, 0)
  family.fontFamily = 'Times New Roman'
  family.textVertical = 'top'
  const saved = parseDesign(serializeDesign({ ...emptyDesign(), screens: [family] })).doc
  assert.equal(saved.screens[0].fontFamily, 'Times New Roman')
  assert.equal(saved.screens[0].textVertical, 'top')
  const row = createNode('frame', 'row', 0, 0)
  row.layout = 'row'
  row.w = 200
  row.h = 100
  row.padTop = 10
  row.padBottom = 10
  row.padLeft = 4
  row.align = 'stretch'
  const body = createNode('rect', 'body', 0, 0)
  body.w = 20
  body.h = 20
  const stay = createNode('rect', 'stay', 0, 0)
  stay.w = 20
  stay.h = 20
  stay.hMode = 'hug'
  row.children = [body, stay]
  const stretched = layoutDesign({ ...emptyDesign(), screens: [row] }).screens[0]
  assert.equal(stretched.children?.[0].x, 4)
  assert.equal(stretched.children?.[0].y, 10)
  assert.equal(stretched.children?.[0].h, 80)
  assert.equal(stretched.children?.[1].h, 20)
  const fixed = createNode('rect', 'fixed', 0, 0)
  fixed.w = 40
  fixed.minW = 70
  const minFrame = createNode('frame', 'min', 0, 0)
  minFrame.layout = 'row'
  minFrame.w = 200
  minFrame.h = 40
  minFrame.children = [fixed]
  assert.equal(layoutDesign({ ...emptyDesign(), screens: [minFrame] }).screens[0].children?.[0].w, 70)
  const wrap = createNode('frame', 'wrap', 0, 0)
  wrap.layout = 'row'
  wrap.wrap = true
  wrap.w = 200
  wrap.h = 200
  const cells = ['a', 'b', 'c'].map((id) => {
    const cell = createNode('rect', id, 0, 0)
    cell.w = 80
    cell.h = 40
    return cell
  })
  wrap.children = cells
  const flowed = layoutDesign({ ...emptyDesign(), screens: [wrap] }).screens[0]
  assert.equal(flowed.children?.[1].x, 80)
  assert.equal(flowed.children?.[2].x, 0)
  assert.equal(flowed.children?.[2].y, 40)
  const open = createNode('frame', 'open', 0, 0)
  open.w = 100
  open.h = 100
  open.clip = false
  const outside = createNode('rect', 'out', -40, 10)
  outside.w = 30
  outside.h = 30
  open.children = [outside]
  const openDoc = { ...emptyDesign(), screens: [open] }
  assert.equal(hitDesign(openDoc, -20, 20)?.id, 'out')
  assert.equal(hitDesign({ ...openDoc, screens: [{ ...open, clip: undefined }] }, -20, 20), null)
  const card = createNode('frame', 'card', 0, 0)
  card.layout = 'row'
  const label = createNode('text', 'label', 0, 0)
  label.text = 'Save'
  const icon = createNode('rect', 'icon', 0, 0)
  card.children = [label, icon]
  const component: DesignComponent = { id: 'card', name: 'Card', variants: [{ props: {}, node: card }] }
  const instance = createNode('frame', 'host', 0, 0)
  const placed: DesignNode = { id: 'inst', kind: 'instance', x: 0, y: 0, w: 80, h: 40, component: 'card', overrides: [{ id: 'label', text: 'Bye' }, { id: 'icon', fill: '#ff00aa', visible: false }] }
  instance.children = [placed]
  const instDoc = { ...emptyDesign(), components: [component], screens: [instance] }
  const resolved = resolveInstanceTree(instDoc, placed)
  assert.equal(resolved?.children?.[0].text, 'Bye')
  assert.equal(resolved?.children?.[1].fill, '#ff00aa')
  assert.equal(resolved?.children?.[1].visible, false)
  const merged = mergeDesignOverride(placed, 'label', { text: null })
  assert.equal(merged.overrides?.some((item) => item.id === 'label'), false)
  const round = parseDesign(serializeDesign(instDoc)).doc
  assert.equal(round.screens[0].children?.[0].overrides?.[1].fill, '#ff00aa')
  const slice = queryDesign(instDoc, { screen: 'host' })
  assert.equal(slice?.screen?.tree.children?.[0].overrides?.[0].text, 'Bye')
  const corners = createNode('rect', 'corners', 0, 0)
  corners.radius = 8
  corners.radiusTL = 0
  corners.radiusTR = 20
  const cornerDoc = parseDesign(serializeDesign({ ...emptyDesign(), screens: [corners] })).doc
  assert.equal(cornerDoc.screens[0].radiusTL, 0)
  assert.equal(cornerDoc.screens[0].radiusTR, 20)
  assert.equal(cornerDoc.screens[0].radius, 8)
  const eff = effectiveInstanceChild(instDoc, placed, 'label')
  assert.equal(eff?.text, 'Bye')
  assert.equal(eff?.hasTextOverride, true)
  const a = createNode('rect', 'a', 0, 0)
  const b = createNode('rect', 'b', 0, 0)
  const flat = { ...emptyDesign(), screens: [a, b] }
  assert.equal(moveDesignNode(flat, 'a', 'b', 0), flat)
  const flowRow = createNode('frame', 'flowRow', 0, 0)
  flowRow.layout = 'row'
  flowRow.w = 100
  flowRow.h = 40
  const kid = createNode('rect', 'kid', 0, 0)
  kid.w = 20
  kid.h = 20
  kid.x = 4
  kid.y = 4
  kid.absolute = true
  flowRow.children = [kid]
  const laid = layoutDesign({ ...emptyDesign(), screens: [flowRow] })
  assert.equal(laid.screens[0].children?.[0].x, 4)
  assert.equal(laid.screens[0].children?.[0].y, 4)
}

{
  const master = createNode('frame', 'btn', 0, 0)
  master.w = 80
  master.h = 40
  master.layout = 'row'
  master.wMode = 'hug'
  const oval = createNode('ellipse', 'dot', 60, -10)
  oval.w = 40
  oval.h = 40
  master.children = [oval]
  const component: DesignComponent = { id: 'btn', name: 'Btn', variants: [{ props: {}, node: master }] }
  const placed = makeInstance({ ...emptyDesign(), components: [component] }, 'btn', 'inst', 0, 0)
  assert.ok(placed)
  assert.equal(placed.w, 80)
  assert.equal(placed.h, 40)
  assert.equal(placed.wMode, 'fixed')
  assert.equal(placed.hMode, 'fixed')
  assert.equal(placed.maxW, 80)
  assert.equal(placed.maxH, 40)
  const visual = resolveInstanceTree({ ...emptyDesign(), components: [component] }, placed)
  assert.equal(visual?.w, 80)
  assert.equal(visual?.h, 40)
  const host = createNode('frame', 'host', 0, 0)
  host.layout = 'column'
  host.align = 'stretch'
  host.w = 240
  host.h = 180
  host.children = [placed]
  const grown = layoutDesign({ ...emptyDesign(), components: [component], screens: [host] }).screens[0]
  assert.equal(grown.children?.[0].w, 80)
  assert.equal(grown.children?.[0].h, 40)
  const filling = { ...placed, wMode: 'fill' as const, hMode: 'fill' as const }
  host.children = [filling]
  const filled = layoutDesign({ ...emptyDesign(), components: [component], screens: [host] }).screens[0]
  assert.equal(filled.children?.[0].w, 80)
  assert.equal(filled.children?.[0].h, 40)
  const huge: DesignNode = { id: 'huge', kind: 'instance', x: 0, y: 0, w: 400, h: 300, component: 'btn' }
  const clamped = layoutDesign({ ...emptyDesign(), components: [component], screens: [huge] }).screens[0]
  assert.equal(clamped.w, 80)
  assert.equal(clamped.h, 40)
  assert.equal(clamped.maxW, 80)
  assert.equal(clamped.maxH, 40)
  const stretched = resizeDesignNode(placed, 'se', 200, 200, 1, false)
  assert.equal(stretched.w, 80)
  assert.equal(stretched.h, 40)
  const page = createNode('frame', 'page', 0, 0)
  page.w = 200
  page.h = 200
  const afterPage = layoutDesign({ ...emptyDesign(), components: [component], screens: [page] })
  assert.equal(afterPage.components[0].variants[0].node.w, 80)
  assert.equal(afterPage.components[0].variants[0].node.h, 40)
}
