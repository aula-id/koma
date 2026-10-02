import { createContext, useContext, useEffect, useState, type JSX } from 'react'
import { useLexicalNodeSelection } from '@lexical/react/useLexicalNodeSelection'
import {
  DecoratorNode,
  type LexicalNode,
  type NodeKey,
  type SerializedLexicalNode,
  type Spread,
} from 'lexical'
import { useKoma } from '../../store/koma'
import { bytesToObjectUrl, requestFileBytes } from '../../lib/filePreview'
import { noteImageFile, safeNoteUrl } from '../../lib/markdownNote'
import { mimeForPath } from '../../lib/viewerKind'

export type NoteAssets = {
  root: string
  assetDir: string
  legacyAssetDir?: string
  previews: Record<string, string>
}

export const NoteAssetsContext = createContext<NoteAssets>({ root: '', assetDir: '', previews: {} })

export type SerializedNoteImageNode = Spread<{ alt: string; src: string }, SerializedLexicalNode>

function NoteImageView({ alt, src, nodeKey }: { alt: string; src: string; nodeKey: NodeKey }) {
  const assets = useContext(NoteAssetsContext)
  const req = useKoma((s) => s.req)
  const [selected, setSelected] = useLexicalNodeSelection(nodeKey)
  const preview = assets.previews[src]
  const [url, setUrl] = useState<string | null>(preview ?? null)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    if (preview) {
      setUrl(preview)
      return
    }
    if (!assets.root || !noteImageFile(src)) return
    let cancelled = false
    let objectUrl: string | null = null
    setUrl(null)
    setFailed(false)
    const load = (dir: string) => requestFileBytes(req, assets.root, `${dir}/${src}`)
    void load(assets.assetDir)
      .catch(() => (assets.legacyAssetDir ? load(assets.legacyAssetDir) : Promise.reject(new Error('missing'))))
      .then((bytes) => {
        if (cancelled) return
        objectUrl = bytesToObjectUrl(bytes, mimeForPath(src))
        setUrl(objectUrl)
      })
      .catch(() => {
        if (!cancelled) setFailed(true)
      })
    return () => {
      cancelled = true
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [assets.assetDir, assets.legacyAssetDir, assets.root, preview, req, src])
  const ring = selected ? 'ring-1 ring-koma-accent' : ''
  const pick = (event: { preventDefault: () => void }) => {
    event.preventDefault()
    setSelected(true)
  }
  if (safeNoteUrl(src) && /^https?:\/\//i.test(src)) {
    return <img src={src} alt={alt} className={`max-w-full rounded ${ring}`} onClick={pick} />
  }
  if (failed) return <span className="text-[11px] text-koma-dim">{alt || src}</span>
  if (!url) return <span className="text-[11px] text-koma-dim">{alt || 'Image'}</span>
  return <img src={url} alt={alt} className={`my-1 max-w-full rounded ${ring}`} onClick={pick} />
}

export class NoteImageNode extends DecoratorNode<JSX.Element> {
  __alt: string
  __src: string

  static getType(): string {
    return 'note-image'
  }

  static clone(node: NoteImageNode): NoteImageNode {
    return new NoteImageNode(node.__alt, node.__src, node.__key)
  }

  static importJSON(serialized: SerializedNoteImageNode): NoteImageNode {
    return $createNoteImageNode(serialized.alt, serialized.src)
  }

  constructor(alt: string, src: string, key?: NodeKey) {
    super(key)
    this.__alt = alt
    this.__src = src
  }

  exportJSON(): SerializedNoteImageNode {
    return { ...super.exportJSON(), type: 'note-image', version: 1, alt: this.__alt, src: this.__src }
  }

  createDOM(): HTMLElement {
    const el = document.createElement('figure')
    el.className = 'my-1'
    return el
  }

  updateDOM(): false {
    return false
  }

  isInline(): boolean {
    return true
  }

  getTextContent(): string {
    return `![${this.__alt}](${this.__src})`
  }

  decorate(): JSX.Element {
    return <NoteImageView alt={this.__alt} src={this.__src} nodeKey={this.getKey()} />
  }
}

export function $createNoteImageNode(alt: string, src: string): NoteImageNode {
  return new NoteImageNode(alt, src)
}

export function $isNoteImageNode(node: LexicalNode | null | undefined): node is NoteImageNode {
  return node instanceof NoteImageNode
}
