import assert from 'node:assert/strict'
import {
  addComponentVariant,
  addDesignToken,
  applyOverrides,
  componentView,
  copyTree,
  createComponentFromFrame,
  createNode,
  deleteDesignNode,
  designChatText,
  designFileName,
  dropDesignToken,
  layoutDesign,
  reorderDesignNode,
  emptyDesign,
  frameAtPoint,
  hitDesign,
  insertDesignNode,
  isDesignPath,
  nodeChrome,
  nodeOrigin,
  parseDesign,
  pickVariant,
  placeDesignNode,
  queryDesign,
  resizeDesignNode,
  resetInstanceOverrides,
  resolveInstanceTree,
  resolveRef,
  serializeDesign,
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
  assert.equal(nodeChrome(frame).fill, '')
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
  const grown = resizeDesignNode(label, 'e', 20, 0, 8, true)
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
