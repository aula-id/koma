import { unzipSync, type Unzipped } from 'fflate'
import type { FigPageManifestEntry } from './kiwi/fig/page-manifest'
import { parseFigKiwiChunks } from './kiwi/fig/container'
import { decodeFigKiwiCanvas } from './kiwi/fig/parse'
import { hasPNGSignature } from './thumbnail'

export interface FigParseResult {
  nodeChanges: Array<Record<string, unknown>>
  blobs: Uint8Array[]
  images: Array<[string, Uint8Array]>
  figKiwiVersion: number
  figSchemaDeflated: Uint8Array
  thumbnailPNG: Uint8Array | null
  metaJSON: string | null
}

function isLikelyAsset(name: string): boolean {
  const lower = name.toLowerCase()
  return lower.endsWith('.png') || lower.endsWith('.jpg') || lower.endsWith('.json')
}

function findCanvasData(entries: Partial<Record<string, Uint8Array>>): Uint8Array | null {
  const canonical = entries['canvas.fig'] ?? entries.canvas
  if (canonical) return canonical
  let largest: Uint8Array | null = null
  for (const [name, data] of Object.entries(entries)) {
    if (!data || isLikelyAsset(name)) continue
    if (!largest || data.byteLength > largest.byteLength) largest = data
  }
  return largest
}

function isCanonicalCanvasEntry(name: string): boolean {
  return name === 'canvas.fig' || name === 'canvas'
}

function parseRawFigKiwi(bytes: Uint8Array, onPages?: (pages: FigPageManifestEntry[]) => void): FigParseResult | null {
  const chunks = parseFigKiwiChunks(bytes)
  if (!chunks) return null
  const decoded = decodeFigKiwiCanvas(bytes, onPages)
  const thumbnailPNG = chunks.slice(2).find(hasPNGSignature) ?? null
  return { ...decoded, images: [], thumbnailPNG, metaJSON: null }
}

/** Parse a zipped or legacy raw `.fig` file. */
export function parseFigBuffer(buffer: ArrayBuffer, onPages?: (pages: FigPageManifestEntry[]) => void): FigParseResult {
  const bytes = new Uint8Array(buffer)
  const raw = parseRawFigKiwi(bytes, onPages)
  if (raw) return raw

  const canvasArchive = unzipSync(bytes, { filter: ({ name }) => isCanonicalCanvasEntry(name) })
  let canvasData = findCanvasData(canvasArchive)
  let archive: Unzipped
  let decoded: ReturnType<typeof decodeFigKiwiCanvas>
  if (canvasData) {
    decoded = decodeFigKiwiCanvas(canvasData, onPages)
    archive = unzipSync(bytes, { filter: ({ name }) => !isCanonicalCanvasEntry(name) })
  } else {
    archive = unzipSync(bytes)
    canvasData = findCanvasData(archive)
    if (!canvasData) {
      throw new Error(`No canvas data found in .fig file. Entries: ${Object.keys(archive).join(', ')}`)
    }
    decoded = decodeFigKiwiCanvas(canvasData, onPages)
  }

  const metaBytes = archive['meta.json']
  const images = Object.entries(archive)
    .filter(([name]) => name.startsWith('images/') && name !== 'images/')
    .map(([name, data]) => [name.slice('images/'.length), data] as [string, Uint8Array])

  return {
    ...decoded,
    images,
    thumbnailPNG: archive['thumbnail.png'] ?? null,
    metaJSON: Object.hasOwn(archive, 'meta.json') ? new TextDecoder().decode(metaBytes) : null,
  }
}
