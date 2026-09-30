import {
  DecoratorNode,
  type LexicalNode,
  type NodeKey,
  type SerializedLexicalNode,
  type Spread,
} from 'lexical'
import type { JSX } from 'react'

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

  decorate(): JSX.Element {
    const file = this.__tone === 'file' ? this.__label.match(/^(@(?:\[\d+\])?)([\s\S]*)$/) : null
    return (
      <span
        className={`rounded px-0.5 ${this.__tone === 'attach' ? 'bg-koma-warn/20 text-koma-fg' : 'bg-koma-accent/15 text-koma-fg'}`}
      >
        {file ? (
          <>
            <span className="opacity-50">{file[1]}</span>
            {file[2]}
          </>
        ) : (
          this.__label
        )}
      </span>
    )
  }
}

export function $createComposerChipNode(label: string, tone: ChipTone): ComposerChipNode {
  return new ComposerChipNode(label, tone)
}

export function $isComposerChipNode(node: LexicalNode | null | undefined): node is ComposerChipNode {
  return node instanceof ComposerChipNode
}
