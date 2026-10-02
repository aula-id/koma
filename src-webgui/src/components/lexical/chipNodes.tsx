import {
  DecoratorNode,
  type LexicalNode,
  type NodeKey,
  type SerializedLexicalNode,
  type Spread,
} from 'lexical'
import { createContext, useContext, type DragEvent, type JSX } from 'react'
import { GripVertical, X } from 'lucide-react'
import { chipPayloadFromWire, type ComposerChipKind, type ComposerChipPayload } from '../../lib/composerIpc'

export const COMPOSER_CHIP_MIME = 'application/x-koma-chip'

export type ComposerChipActions = {
  onPasteChipDoubleClick?: (markerN: number) => void
  onImageChipDoubleClick?: (markerN: number) => void
  onFileChipDoubleClick?: (wireText: string) => void
  onRemoveChip?: (payload: {
    nodeKey: NodeKey
    kind: ComposerChipKind
    markerN: number | null
    wireText: string
  }) => void
}

export const ComposerChipContext = createContext<ComposerChipActions>({})

export type SerializedComposerChipNode = Spread<
  {
    wireText: string
    displayLabel: string
    kind: ComposerChipKind
    markerN?: number | null
    queueId?: string | null
    /** @deprecated legacy */
    label?: string
    tone?: 'file' | 'attach'
  },
  SerializedLexicalNode
>

/** Atomic composer token. Export uses `wireText`; UI shows `displayLabel`. */
export class ComposerChipNode extends DecoratorNode<JSX.Element> {
  __wireText: string
  __displayLabel: string
  __kind: ComposerChipKind
  __markerN: number | null
  __queueId: string | null

  static getType(): string {
    return 'composer-chip'
  }

  static clone(node: ComposerChipNode): ComposerChipNode {
    return new ComposerChipNode(
      {
        kind: node.__kind,
        wireText: node.__wireText,
        displayLabel: node.__displayLabel,
        markerN: node.__markerN,
        queueId: node.__queueId,
      },
      node.__key,
    )
  }

  static importJSON(serialized: SerializedComposerChipNode): ComposerChipNode {
    if (serialized.wireText) {
      return $createComposerChipNode({
        kind: serialized.kind,
        wireText: serialized.wireText,
        displayLabel: serialized.displayLabel,
        markerN: serialized.markerN ?? null,
        queueId: serialized.queueId ?? null,
      })
    }
    const wire = serialized.label ?? ''
    const legacyTone = serialized.tone ?? 'file'
    const payload = chipPayloadFromWire(wire)
    if (legacyTone === 'file') payload.kind = 'file'
    else if (payload.kind === 'file') payload.kind = wire.includes('Pasted Text') ? 'paste' : 'image'
    return $createComposerChipNode(payload)
  }

  constructor(payload: ComposerChipPayload, key?: NodeKey) {
    super(key)
    this.__wireText = payload.wireText
    this.__displayLabel = payload.displayLabel
    this.__kind = payload.kind
    this.__markerN = payload.markerN ?? null
    this.__queueId = payload.queueId ?? null
  }

  exportJSON(): SerializedComposerChipNode {
    return {
      ...super.exportJSON(),
      type: 'composer-chip',
      version: 2,
      wireText: this.__wireText,
      displayLabel: this.__displayLabel,
      kind: this.__kind,
      markerN: this.__markerN,
      queueId: this.__queueId,
    }
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
    return this.__wireText
  }

  getWireText(): string {
    return this.__wireText
  }

  getDisplayLabel(): string {
    return this.__displayLabel
  }

  getChipKind(): ComposerChipKind {
    return this.__kind
  }

  getMarkerN(): number | null {
    return this.__markerN
  }

  getQueueId(): string | null {
    return this.__queueId
  }

  setMarkerWire(markerN: number, wireText: string, displayLabel: string): void {
    const self = this.getWritable()
    self.__markerN = markerN
    self.__wireText = wireText
    self.__displayLabel = displayLabel
  }

  decorate(): JSX.Element {
    return (
      <ComposerChipView
        displayLabel={this.__displayLabel}
        kind={this.__kind}
        markerN={this.__markerN}
        wireText={this.__wireText}
        nodeKey={this.getKey()}
      />
    )
  }
}

function kindClass(kind: ComposerChipKind): string {
  if (kind === 'file') return 'border-koma-accent/30 bg-koma-accent/15 text-koma-fg'
  if (kind === 'image') return 'border-koma-warn/40 bg-koma-warn/20 text-koma-fg'
  return 'border-koma-warn/50 bg-koma-warn/25 text-koma-fg'
}

function ComposerChipView({
  displayLabel,
  kind,
  markerN,
  wireText,
  nodeKey,
}: {
  displayLabel: string
  kind: ComposerChipKind
  markerN: number | null
  wireText: string
  nodeKey: NodeKey
}) {
  const actions = useContext(ComposerChipContext)

  const onDragStart = (event: DragEvent) => {
    event.stopPropagation()
    event.dataTransfer.setData(COMPOSER_CHIP_MIME, nodeKey)
    event.dataTransfer.setData('text/plain', `koma-chip:${nodeKey}`)
    event.dataTransfer.effectAllowed = 'move'
  }

  const onDoubleClick = () => {
    if (kind === 'paste' && markerN != null) actions.onPasteChipDoubleClick?.(markerN)
    else if (kind === 'image' && markerN != null) actions.onImageChipDoubleClick?.(markerN)
    else if (kind === 'file') actions.onFileChipDoubleClick?.(wireText)
  }

  const title =
    kind === 'paste'
      ? 'Double-click to edit paste'
      : kind === 'image'
        ? 'Double-click to view image'
        : kind === 'file'
          ? 'Double-click to open file'
          : 'Drag grip to move'

  return (
    <span
      contentEditable={false}
      title={title}
      onDoubleClick={onDoubleClick}
      className={`inline-flex max-w-full items-center gap-0.5 rounded-md border px-1 py-0.5 align-baseline text-[11px] ${kindClass(kind)}`}
    >
      <span
        draggable
        onDragStart={onDragStart}
        onPointerDown={(event) => event.stopPropagation()}
        className="cursor-grab active:cursor-grabbing"
        aria-hidden
      >
        <GripVertical size={11} className="opacity-50" />
      </span>
      <span className="truncate">{displayLabel}</span>
      <button
        type="button"
        title="Remove"
        aria-label={`Remove ${displayLabel}`}
        onPointerDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          event.stopPropagation()
          actions.onRemoveChip?.({ nodeKey, kind, markerN, wireText })
        }}
        className="flex-none rounded p-0.5 opacity-50 hover:bg-koma-hover hover:opacity-100"
      >
        <X size={10} />
      </button>
    </span>
  )
}

export function $createComposerChipNode(payload: ComposerChipPayload): ComposerChipNode {
  return new ComposerChipNode(payload)
}

export function $isComposerChipNode(node: LexicalNode | null | undefined): node is ComposerChipNode {
  return node instanceof ComposerChipNode
}
