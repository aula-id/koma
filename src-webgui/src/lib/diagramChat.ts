import { useKoma } from '../store/koma'
import { fileKey, type FileReadPush } from '../store/coding'
import { codingRequest } from './coding-service'
import { parseDiagram, type DiagramDoc } from './diagram'
import {
  captureDiagram,
  diagramToMermaid,
  rememberDiagramView,
  type DiagramRect,
} from './diagramMermaid'
import { attachDiagramShapesToComposer } from './diagramNoteAttach'

function toast(text: string, kind: 'info' | 'error' = 'info') {
  useKoma.setState((s) => {
    const id = s.ui.toastSeq + 1
    return { ui: { ...s.ui, toastSeq: id, toast: { id, text, kind } } }
  })
}

export function diagramChatTitle(path: string): string {
  const base = path.split(/[/\\]/).pop() || path
  return base.replace(/\.diag$/i, '') || base
}

async function resolveDiagramDoc(root: string, path: string): Promise<DiagramDoc> {
  const open = useKoma.getState().diagram.docs[fileKey(root, path)]
  if (open?.loading) throw new Error('Wait for this diagram to finish loading.')
  if (open?.error && open.savedText == null) throw new Error(open.error)
  if (open && !open.error) return open.doc
  const hostId = useKoma.getState().remoteState.hostId ?? 'local'
  const value = await codingRequest<Omit<FileReadPush, 'k'>>({ hostId, root }, { op: 'read', path })
  if (value.error || value.binary || value.tooLarge || value.content == null) {
    throw new Error(value.error ?? 'This diagram cannot be read as text.')
  }
  const parsed = parseDiagram(value.content)
  if (parsed.error) throw new Error(parsed.error)
  return parsed.doc
}

type DiagramSource = { root: string; path: string; notes?: DiagramDoc }

function publish(doc: DiagramDoc, title: string, empty: string, source?: DiagramSource): string | null {
  const mermaid = diagramToMermaid(doc, title)
  if (!mermaid) {
    toast(empty)
    return null
  }
  rememberDiagramView(mermaid, doc)
  useKoma.getState().addDiagramToChat({ title, mermaid, doc })
  if (source) {
    void attachDiagramShapesToComposer(source.root, source.path, source.notes ?? doc)
  }
  return mermaid
}

export function addDiagramDocToChat(doc: DiagramDoc, title: string, empty = 'This diagram is empty.', source?: DiagramSource) {
  publish(doc, title, empty, source)
}

export async function copyDiagramMermaid(doc: DiagramDoc, title: string, empty = 'This diagram is empty.', source?: DiagramSource) {
  const mermaid = diagramToMermaid(doc, title)
  if (!mermaid) {
    toast(empty)
    return
  }
  try {
    await navigator.clipboard.writeText(mermaid)
    toast('Copied Mermaid')
  } catch (error) {
    toast(error instanceof Error ? error.message : 'Could not copy Mermaid', 'error')
  }
}

export async function addDiagramFileToChat(root: string, path: string) {
  try {
    const doc = await resolveDiagramDoc(root, path)
    addDiagramDocToChat(doc, diagramChatTitle(path), 'This diagram is empty.', { root, path })
  } catch (error) {
    toast(error instanceof Error ? error.message : 'Could not read this diagram', 'error')
  }
}

export async function copyDiagramFileMermaid(root: string, path: string) {
  try {
    const doc = await resolveDiagramDoc(root, path)
    await copyDiagramMermaid(doc, diagramChatTitle(path), 'This diagram is empty.', { root, path })
  } catch (error) {
    toast(error instanceof Error ? error.message : 'Could not read this diagram', 'error')
  }
}

export function addDiagramAreaToChat(doc: DiagramDoc, area: DiagramRect, title: string, source?: DiagramSource) {
  const captured = captureDiagram(doc, area)
  addDiagramDocToChat(
    captured,
    `${title} selection`,
    'Nothing in that area.',
    source ? { ...source, notes: captured } : undefined,
  )
}

export function copyDiagramArea(doc: DiagramDoc, area: DiagramRect, title: string, source?: DiagramSource) {
  return copyDiagramMermaid(captureDiagram(doc, area), `${title} selection`, 'Nothing in that area.', source ? { ...source, notes: doc } : undefined)
}
