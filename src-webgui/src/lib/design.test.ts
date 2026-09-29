import assert from 'node:assert/strict'
import {
  applyOverrides,
  copyTree,
  createNode,
  designFileName,
  emptyDesign,
  isDesignPath,
  nodeChrome,
  parseDesign,
  pickVariant,
  queryDesign,
  resolveRef,
  serializeDesign,
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
