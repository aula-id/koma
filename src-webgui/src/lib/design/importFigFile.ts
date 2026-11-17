import { designFileName, importFigToDesign, serializeDesign, writeDesignAsset } from '../design'
import { utf8ToBase64 } from '../diagramNotes'
import { mintRequestId } from '../../store/coding'
import type { KomaState } from '../../store/state'

/** Imports a .fig file into a new .koma design file of `root` and opens it. */
export async function importFigFile(
  file: File,
  root: string,
  req: KomaState['req'],
  openDesignTab: (root: string, path: string) => void,
): Promise<void> {
  const buffer = await file.arrayBuffer()
  const imported = await importFigToDesign(buffer)
  if (imported.error) return
  for (const image of imported.images ?? []) writeDesignAsset(req, root, image.path, image.bytes)
  const name = designFileName(file.name.replace(/\.fig$/i, '')) ?? `import-${Date.now()}.kdsgn`
  const path = name.startsWith('.koma/') ? name : `.koma/${name}`
  req({
    r: 'FileWriteBytes',
    root,
    path,
    bytesB64: utf8ToBase64(serializeDesign(imported.doc)),
    overwrite: false,
    requestId: mintRequestId(),
  })
  openDesignTab(root, path)
}
