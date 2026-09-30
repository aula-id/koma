import { type DesignDoc, type DesignNode, type DesignTokenKind } from './types'
import {
  designLayerName,
  pickVariant,
  variantKey,
} from './model'
import { writeNode } from './serialize'

export type DesignQuery =
  | { tokens: true }
  | { component: string; variant?: Record<string, string> }
  | { screen: string }

export type DesignTokenSlice = {
  name: string
  kind: DesignTokenKind
  values: Record<string, string>
}

export type DesignQuerySlice = {
  mode: string
  modes: string[]
  tokens: DesignTokenSlice[]
  component?: {
    id: string
    name: string
    axes: Record<string, string[]>
    variant: Record<string, string>
    tree: DesignNode
  }
  screen?: {
    id: string
    name: string
    tree: DesignNode
  }
  components?: { id: string; name: string; axes: Record<string, string[]>; variant: Record<string, string> }[]
}

function tokenRefs(node: DesignNode, into: Set<string>) {
  for (const field of [node.fill, node.stroke, node.color]) {
    if (field && field !== 'none' && !field.startsWith('#')) into.add(field)
  }
  if (typeof node.radius === 'string') into.add(node.radius)
  for (const corner of [node.radiusTL, node.radiusTR, node.radiusBR, node.radiusBL]) {
    if (typeof corner === 'string') into.add(corner)
  }
  for (const row of node.overrides ?? []) {
    if (row.fill && row.fill !== 'none' && !row.fill.startsWith('#')) into.add(row.fill)
  }
  for (const child of node.children ?? []) tokenRefs(child, into)
}

function tokenSlices(doc: DesignDoc, names: Set<string>): DesignTokenSlice[] {
  return doc.tokens
    .filter((token) => names.has(token.name))
    .map((token) => ({ name: token.name, kind: token.kind, values: { ...token.values } }))
}

function sliceBase(doc: DesignDoc, names: Set<string>): DesignQuerySlice {
  return { mode: doc.mode, modes: [...doc.modes], tokens: tokenSlices(doc, names) }
}

/**
 * One compact slice for the model. A token query has no screens.
 * A component query is that variant's tree. A screen query keeps instances
 * as instances and names the components they use. It never returns the file.
 */
export function queryDesign(doc: DesignDoc, query: DesignQuery): DesignQuerySlice | null {
  if ('tokens' in query) return sliceBase(doc, new Set(doc.tokens.map((token) => token.name)))
  if ('component' in query) {
    const component = doc.components.find((item) => item.id === query.component)
    if (!component) return null
    const variant = pickVariant(component, query.variant)
    if (!variant) return null
    const names = new Set<string>()
    tokenRefs(variant.node, names)
    return {
      ...sliceBase(doc, names),
      component: {
        id: component.id,
        name: component.name,
        axes: component.axes ? { ...component.axes } : {},
        variant: { ...variant.props },
        tree: writeNode(variant.node),
      },
    }
  }
  const screen = doc.screens.find((item) => item.id === query.screen || item.name === query.screen)
  if (!screen) return null
  const names = new Set<string>()
  tokenRefs(screen, names)
  const used: DesignQuerySlice['components'] = []
  const seen = new Set<string>()
  const visit = (node: DesignNode) => {
    if (node.kind === 'instance' && node.component && !seen.has(`${node.component}:${variantKey(node.variant)}`)) {
      seen.add(`${node.component}:${variantKey(node.variant)}`)
      const component = doc.components.find((item) => item.id === node.component)
      if (component) {
        const variant = pickVariant(component, node.variant)
        if (variant) tokenRefs(variant.node, names)
        used.push({
          id: component.id,
          name: component.name,
          axes: component.axes ? { ...component.axes } : {},
          variant: { ...(variant?.props ?? node.variant ?? {}) },
        })
      }
    }
    for (const child of node.children ?? []) visit(child)
  }
  visit(screen)
  return {
    ...sliceBase(doc, names),
    screen: { id: screen.id, name: screen.name ?? 'Screen', tree: writeNode(screen) },
    components: used,
  }
}

/** One fenced slice for chat. The fence is the model payload. */
export function designChatText(doc: DesignDoc, query: DesignQuery): string | null {
  const slice = queryDesign(doc, query)
  if (!slice) return null
  return '```kdsgn\n' + JSON.stringify(slice) + '\n```'
}

/** The screen or component tree a chat query names. */
export function designQueryNode(doc: DesignDoc, query: DesignQuery): DesignNode | null {
  if ('tokens' in query) return null
  if ('component' in query) {
    const component = doc.components.find((item) => item.id === query.component)
    if (!component) return null
    return pickVariant(component, query.variant)?.node ?? null
  }
  return doc.screens.find((item) => item.id === query.screen || item.name === query.screen) ?? null
}

/** Parent-relative box for each node, so a picture can be read against positions. */
export function designCoordinateText(doc: DesignDoc, node: DesignNode): string {
  const lines: string[] = []
  const walk = (item: DesignNode, depth: number) => {
    if (lines.length >= 80) return
    const detail = [
      item.kind === 'text' && item.text ? JSON.stringify(item.text) : '',
      item.kind === 'instance' ? doc.components.find((component) => component.id === item.component)?.name ?? item.component ?? '' : '',
      item.fill && item.fill !== 'none' ? item.fill : '',
      item.flipX ? 'flipX' : '',
      item.flipY ? 'flipY' : '',
    ].filter(Boolean).join(' ')
    lines.push(`${'  '.repeat(depth)}${designLayerName(item)} ${Math.round(item.w)}×${Math.round(item.h)} at (${Math.round(item.x)}, ${Math.round(item.y)})${detail ? ` ${detail}` : ''}`)
    for (const child of item.children ?? []) walk(child, depth + 1)
  }
  walk(node, 0)
  return lines.join('\n')
}

/** Coordinate list plus the fenced slice. Prefer `designChatText` for chat chips. */
export function designChatNote(doc: DesignDoc, query: DesignQuery): string | null {
  const fence = designChatText(doc, query)
  const node = designQueryNode(doc, query)
  if (!fence || !node) return null
  return `Attached image is the render. Coordinates are parent-relative pixels.\n${designCoordinateText(doc, node)}\n\n${fence}`
}

export function designChatTitle(doc: DesignDoc, query: DesignQuery): string {
  if ('component' in query) return doc.components.find((item) => item.id === query.component)?.name ?? 'Component'
  const node = designQueryNode(doc, query)
  return node ? designLayerName(node) : 'Design'
}

const KDSGN_FENCE = /```[ \t]*kdsgn[ \t]*\r?\n[\s\S]*?```/gi

/** Title shown on a design chip. The fence body is one query slice. */
export function designFenceTitle(fence: string): string {
  const body = fence.replace(/^```[ \t]*kdsgn[ \t]*\r?\n/, '').replace(/```\s*$/, '')
  try {
    const value = JSON.parse(body) as { screen?: { name?: string }; component?: { name?: string } }
    const title = value.screen?.name || value.component?.name
    return title || 'Design'
  } catch {
    return 'Design'
  }
}

/** Drop legacy coordinate dumps; chips + PNG carry design context now. */
function stripLegacyDesignCoordinateProse(prose: string): string {
  const rest = prose.split('\n').filter((line) => {
    if (/^Attached image is the render\./.test(line.trim())) return false
    if (/\d+[×x]\d+ at \(/i.test(line)) return false
    return true
  })
  return rest.join('\n').replace(/\n{3,}/g, '\n\n').trim()
}

/** Pull fenced design slices out of a user message. The fence is what the model read. */
export function splitDesignMessage(content: string): { prose: string; designs: { text: string; title: string }[] } {
  const designs: { text: string; title: string }[] = []
  const prose = content.replace(new RegExp(KDSGN_FENCE.source, 'gi'), (fence) => {
    designs.push({ text: fence, title: designFenceTitle(fence) })
    return ''
  })
  const trimmed = prose.replace(/\n{3,}/g, '\n\n').trim()
  return { prose: stripLegacyDesignCoordinateProse(trimmed), designs }
}
