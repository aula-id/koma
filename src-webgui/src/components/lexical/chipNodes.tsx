import {
  DecoratorNode,
  type LexicalNode,
  type NodeKey,
  type SerializedLexicalNode,
  type Spread,
} from 'lexical'
import type { JSX } from 'react'

export const COMPOSER_CHIP_MIME = 'application/x-koma-chip'

export type ChipTone = 'file' | 'attach'

export type SerializedComposerChipNode = Spread<
  { label: string; tone: ChipTone },
  SerializedLexicalNode
>

/** Atomic composer token. The markdown export is the label itself (`@path`, `[Image #N]`). */
export class ComposerChipNode extends DecoratorNode<JSX.Element> {
  __label: string
  __tone: ChipTone

  static getType(): string {
    return 'composer-chip'
  }

  static clone(node: ComposerChipNode): ComposerChipNode {
    return new ComposerChipNode(node.__label, node.__tone, node.__key)
  }

  static importJSON(serialized: SerializedComposerChipNode): ComposerChipNode {
    return $createComposerChipNode(serialized.label, serialized.tone)
  }

  constructor(label: string, tone: ChipTone, key?: NodeKey) {
    super(key)
    this.__label = label
    this.__tone = tone
  }

  exportJSON(): SerializedComposerChipNode {
    return { ...super.exportJSON(), type: 'composer-chip', version: 1, label: this.__label, tone: this.__tone }
  }

  createDOM(): HTMLElement {
    return document.createElement('span')
  }

  updateDOM(): false {
    return false
  }

  isInline(): boolean {
    return true
  }

  isIsolated(): boolean {
    return true
  }

  getTextContent(): string {
    return this.__label
  }

  getTone(): ChipTone {
    return this.__tone
  }

  decorate(): JSX.Element {
    return <ComposerChipView label={this.__label} tone={this.__tone} nodeKey={this.getKey()} />
  }
}

function ComposerChipView({ label, tone, nodeKey }: { label: string; tone: ChipTone; nodeKey: NodeKey }) {
  const file = tone === 'file' ? label.match(/^(@(?:\[\d+\])?)([\s\S]*)$/) : null
  return (
    <span
      draggable
      contentEditable={false}
      title="Drag to move"
      className={`cursor-grab rounded px-0.5 active:cursor-grabbing ${tone === 'attach' ? 'bg-koma-warn/20 text-koma-fg' : 'bg-koma-accent/15 text-koma-fg'}`}
      onMouseDown={(event) => event.stopPropagation()}
      onPointerDown={(event) => event.stopPropagation()}
      onDragStart={(event) => {
        event.stopPropagation()
        event.dataTransfer.setData(COMPOSER_CHIP_MIME, nodeKey)
        event.dataTransfer.setData('text/plain', `koma-chip:${nodeKey}`)
        event.dataTransfer.effectAllowed = 'move'
      }}
    >
      {file ? (
        <>
          <span className="opacity-50">{file[1]}</span>
          {file[2]}
        </>
      ) : (
        label
      )}
    </span>
  )
}

export function $createComposerChipNode(label: string, tone: ChipTone): ComposerChipNode {
  return new ComposerChipNode(label, tone)
}

export function $isComposerChipNode(node: LexicalNode | null | undefined): node is ComposerChipNode {
  return node instanceof ComposerChipNode
}
