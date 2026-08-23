/**
 * Non-text coding file preview (image / pdf / video / sqlite / docx / excel).
 * Bytes come from FileDownloadBytes via filePreview.ts.
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type PointerEvent as ReactPointerEvent,
} from 'react'
import { Download, Maximize2, Minus, Plus, RotateCcw } from 'lucide-react'
import { unzipSync, strFromU8 } from 'fflate'
import { useKoma } from '../store/koma'
import {
  bytesToObjectUrl,
  requestFileBytes,
} from '../lib/filePreview'
import {
  mimeForPath,
  viewerKindForPath,
  type ViewerKind,
} from '../lib/viewerKind'
import { BrailleSpinner } from './BrailleSpinner'

type Props = {
  root: string
  path: string
  /** Optional chrome path label already shown by parent. */
  onDownload?: () => void
}

export function CodingFileViewer({ root, path, onDownload }: Props) {
  const kind = viewerKindForPath(path)
  const req = useKoma((s) => s.req)
  const downloadCodingFile = useKoma((s) => s.downloadCodingFile)
  const [bytes, setBytes] = useState<Uint8Array | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [objectUrl, setObjectUrl] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    setLoading(true)
    setError(null)
    setBytes(null)
    setObjectUrl((prev) => {
      if (prev) URL.revokeObjectURL(prev)
      return null
    })
    void requestFileBytes((body) => req(body as never), root, path)
      .then((b) => {
        if (cancelled) return
        setBytes(b)
        if (kind === 'image' || kind === 'pdf' || kind === 'video') {
          const url = bytesToObjectUrl(b, mimeForPath(path))
          setObjectUrl(url)
        }
        setLoading(false)
      })
      .catch((e: Error) => {
        if (cancelled) return
        setError(e.message || 'failed to load preview')
        setLoading(false)
      })
    return () => {
      cancelled = true
      setObjectUrl((prev) => {
        if (prev) URL.revokeObjectURL(prev)
        return null
      })
    }
  }, [root, path, kind, req])

  const doDownload = () => {
    if (onDownload) onDownload()
    else downloadCodingFile(root, path)
  }

  if (loading) {
    return (
      <Centered>
        <BrailleSpinner size={16} className="opacity-70" />
        <span>Loading preview…</span>
      </Centered>
    )
  }
  if (error) {
    return (
      <Centered>
        <div>{error}</div>
        <DownloadBtn onClick={doDownload} />
      </Centered>
    )
  }
  if (!bytes) {
    return (
      <Centered>
        <div>No data</div>
        <DownloadBtn onClick={doDownload} />
      </Centered>
    )
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-7 flex-none items-center justify-end gap-2 border-b border-koma-border bg-koma-panel2 px-2">
        <span className="mr-auto truncate text-[11px] text-koma-dim">{kindLabel(kind)}</span>
        <button
          type="button"
          onClick={doDownload}
          className="flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
        >
          <Download size={12} />
          Download
        </button>
      </div>
      <div className="relative min-h-0 flex-1 overflow-auto">
        {kind === 'image' && objectUrl ? (
          <ImageBody url={objectUrl} path={path} />
        ) : kind === 'pdf' && objectUrl ? (
          <iframe
            title={path}
            src={objectUrl}
            className="absolute inset-0 h-full w-full border-0 bg-koma-bg"
          />
        ) : kind === 'video' && objectUrl ? (
          <div className="flex h-full min-h-[200px] items-center justify-center bg-black/40 p-4">
            <video
              src={objectUrl}
              controls
              className="max-h-full max-w-full"
            />
          </div>
        ) : kind === 'sqlite' ? (
          <SqliteBody bytes={bytes} />
        ) : kind === 'docx' ? (
          <DocxBody bytes={bytes} />
        ) : kind === 'excel' ? (
          <ExcelBody bytes={bytes} />
        ) : (
          <Centered>
            <div>No preview for this type</div>
            <DownloadBtn onClick={doDownload} />
          </Centered>
        )}
      </div>
    </div>
  )
}

function kindLabel(k: ViewerKind): string {
  switch (k) {
    case 'image':
      return 'Image'
    case 'pdf':
      return 'PDF'
    case 'video':
      return 'Video'
    case 'sqlite':
      return 'SQLite'
    case 'docx':
      return 'Word'
    case 'excel':
      return 'Excel'
    default:
      return 'Preview'
  }
}

function Centered({ children }: { children: ReactNode }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-6 text-center text-[12px] text-koma-dim">
      {children}
    </div>
  )
}

function DownloadBtn({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-center gap-1 rounded border border-koma-border px-2 py-1 text-[11.5px] text-koma-fg hover:bg-koma-hover"
    >
      <Download size={12} />
      Download
    </button>
  )
}

const ZOOM_MIN = 0.1
const ZOOM_MAX = 8
const ZOOM_STEP = 0.25

function clampZoom(z: number): number {
  return Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, z))
}

function ImageBody({ url, path }: { url: string; path: string }) {
  const viewportRef = useRef<HTMLDivElement>(null)
  const [natural, setNatural] = useState<{ w: number; h: number } | null>(null)
  const [zoom, setZoom] = useState(1)
  const [offset, setOffset] = useState({ x: 0, y: 0 })
  const [mode, setMode] = useState<'fit' | 'custom'>('fit')
  const dragRef = useRef<{
    pointerId: number
    startX: number
    startY: number
    origX: number
    origY: number
  } | null>(null)

  // Reset when the image source changes.
  useEffect(() => {
    setNatural(null)
    setZoom(1)
    setOffset({ x: 0, y: 0 })
    setMode('fit')
  }, [url])

  const fitZoom = useCallback((): number => {
    const el = viewportRef.current
    if (!el || !natural) return 1
    const pad = 32
    const availW = Math.max(1, el.clientWidth - pad)
    const availH = Math.max(1, el.clientHeight - pad)
    return clampZoom(Math.min(availW / natural.w, availH / natural.h, 1))
  }, [natural])

  // Keep fit mode locked to the viewport size.
  useEffect(() => {
    if (mode !== 'fit' || !natural) return
    const el = viewportRef.current
    if (!el) return
    const apply = () => {
      setZoom(fitZoom())
      setOffset({ x: 0, y: 0 })
    }
    apply()
    const ro = new ResizeObserver(apply)
    ro.observe(el)
    return () => ro.disconnect()
  }, [mode, natural, fitZoom])

  const zoomRef = useRef(zoom)
  zoomRef.current = zoom
  const offsetRef = useRef(offset)
  offsetRef.current = offset

  const setZoomAround = useCallback(
    (next: number, clientX?: number, clientY?: number) => {
      const prev = zoomRef.current
      const z = clampZoom(next)
      if (z === prev && mode === 'custom') return
      const el = viewportRef.current
      setMode('custom')
      if (!el || !natural || prev <= 0) {
        setZoom(z)
        return
      }
      const rect = el.getBoundingClientRect()
      const cx = clientX ?? rect.left + rect.width / 2
      const cy = clientY ?? rect.top + rect.height / 2
      const vx = cx - rect.left - rect.width / 2
      const vy = cy - rect.top - rect.height / 2
      const off = offsetRef.current
      // Point under cursor stays put: content = (screen - offset) / zoom
      const contentX = (vx - off.x) / prev
      const contentY = (vy - off.y) / prev
      setOffset({
        x: vx - contentX * z,
        y: vy - contentY * z,
      })
      setZoom(z)
    },
    [natural, mode],
  )

  const zoomIn = () => setZoomAround(zoomRef.current + ZOOM_STEP)
  const zoomOut = () => setZoomAround(zoomRef.current - ZOOM_STEP)
  const zoomActual = () => {
    setMode('custom')
    setZoom(1)
    setOffset({ x: 0, y: 0 })
  }
  const zoomFit = () => {
    setMode('fit')
    setZoom(fitZoom())
    setOffset({ x: 0, y: 0 })
  }

  // Non-passive wheel so preventDefault actually stops page/tab scroll.
  useEffect(() => {
    const el = viewportRef.current
    if (!el) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      e.stopPropagation()
      const factor = e.deltaY > 0 ? 1 / 1.12 : 1.12
      setZoomAround(zoomRef.current * factor, e.clientX, e.clientY)
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [setZoomAround])

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    // Only pan when zoomed past fit.
    if (mode === 'fit' && zoom <= fitZoom() + 0.001) return
    e.currentTarget.setPointerCapture(e.pointerId)
    dragRef.current = {
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      origX: offset.x,
      origY: offset.y,
    }
  }
  const onPointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = dragRef.current
    if (!d || d.pointerId !== e.pointerId) return
    setOffset({
      x: d.origX + (e.clientX - d.startX),
      y: d.origY + (e.clientY - d.startY),
    })
  }
  const endDrag = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = dragRef.current
    if (!d || d.pointerId !== e.pointerId) return
    dragRef.current = null
    try {
      e.currentTarget.releasePointerCapture(e.pointerId)
    } catch {
      /* ignore */
    }
  }

  const pct = Math.round(zoom * 100)
  const canPan = mode === 'custom' || zoom > (natural ? fitZoom() : 1) + 0.001

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-7 flex-none items-center gap-0.5 border-b border-koma-border bg-koma-panel2 px-1.5">
        <ToolBtn title="Zoom out" onClick={zoomOut} disabled={zoom <= ZOOM_MIN}>
          <Minus size={12} />
        </ToolBtn>
        <button
          type="button"
          title="Click to toggle fit / 100%"
          onClick={() => (mode === 'fit' ? zoomActual() : zoomFit())}
          className="min-w-[3.25rem] rounded px-1.5 py-0.5 text-center font-mono text-[11px] tabular-nums text-koma-fg hover:bg-koma-hover"
        >
          {pct}%
        </button>
        <ToolBtn title="Zoom in" onClick={zoomIn} disabled={zoom >= ZOOM_MAX}>
          <Plus size={12} />
        </ToolBtn>
        <div className="mx-1 h-3 w-px bg-koma-border" />
        <ToolBtn title="Fit to window" onClick={zoomFit} active={mode === 'fit'}>
          <Maximize2 size={12} />
        </ToolBtn>
        <ToolBtn
          title="Actual size (100%)"
          onClick={zoomActual}
          active={mode === 'custom' && Math.abs(zoom - 1) < 0.01}
        >
          <span className="text-[10px] font-medium leading-none">1:1</span>
        </ToolBtn>
        <ToolBtn title="Reset" onClick={zoomFit}>
          <RotateCcw size={12} />
        </ToolBtn>
        {natural ? (
          <span className="ml-auto truncate px-1 font-mono text-[10.5px] text-koma-dim">
            {natural.w}×{natural.h}
          </span>
        ) : null}
      </div>
      <div
        ref={viewportRef}
        className={`relative min-h-0 flex-1 overflow-hidden bg-[repeating-conic-gradient(color-mix(in_srgb,var(--color-koma-fg)_6%,transparent)_0%_25%,transparent_0%_50%)] bg-[length:16px_16px] ${
          canPan ? 'cursor-grab active:cursor-grabbing' : 'cursor-default'
        }`}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
      >
        <div
          className="absolute left-1/2 top-1/2 will-change-transform"
          style={{
            transform: `translate(calc(-50% + ${offset.x}px), calc(-50% + ${offset.y}px)) scale(${zoom})`,
            transformOrigin: 'center center',
          }}
        >
          <img
            src={url}
            alt={path}
            draggable={false}
            className="block max-w-none select-none shadow-lg"
            style={{
              width: natural ? natural.w : undefined,
              height: natural ? natural.h : undefined,
            }}
            onLoad={(e) => {
              const img = e.currentTarget
              setNatural({ w: img.naturalWidth, h: img.naturalHeight })
            }}
          />
        </div>
      </div>
    </div>
  )
}

function ToolBtn({
  children,
  onClick,
  title,
  disabled,
  active,
}: {
  children: ReactNode
  onClick: () => void
  title: string
  disabled?: boolean
  active?: boolean
}) {
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={onClick}
      className={`flex h-5 min-w-5 items-center justify-center rounded px-1 text-koma-dim transition-colors hover:bg-koma-hover hover:text-koma-fg disabled:cursor-default disabled:opacity-30 ${
        active ? 'bg-koma-hover text-koma-fg' : ''
      }`}
    >
      {children}
    </button>
  )
}

// ─── SQLite (header + page scan for table names; no full SQL engine) ─────────

function SqliteBody({ bytes }: { bytes: Uint8Array }) {
  const info = useMemo(() => parseSqliteLite(bytes), [bytes])
  if (info.error) {
    return (
      <div className="p-4 text-[12px] text-koma-dim">
        <div className="mb-2 text-koma-error">{info.error}</div>
        <div>Download the file and open it with a SQLite client for full browsing.</div>
      </div>
    )
  }
  return (
    <div className="flex flex-col gap-3 p-3 text-[12px] text-koma-fg">
      <div className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-[11.5px]">
        <span className="text-koma-dim">Page size</span>
        <span className="font-mono">{info.pageSize}</span>
        <span className="text-koma-dim">Pages</span>
        <span className="font-mono">{info.pageCount}</span>
        <span className="text-koma-dim">Encoding</span>
        <span className="font-mono">{info.encoding}</span>
        <span className="text-koma-dim">Size</span>
        <span className="font-mono">{formatBytes(bytes.byteLength)}</span>
      </div>
      <div>
        <div className="mb-1 text-[10px] uppercase tracking-wider text-koma-dim">
          Tables (scanned)
        </div>
        {info.tables.length === 0 ? (
          <div className="text-koma-dim">No table names found in a quick scan.</div>
        ) : (
          <ul className="divide-y divide-koma-border rounded border border-koma-border">
            {info.tables.map((t) => (
              <li key={t} className="truncate px-2 py-1 font-mono text-[11.5px]">
                {t}
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="text-[11px] text-koma-dim">
        Read-only header preview — row browsing needs a full SQL engine (not bundled).
      </div>
    </div>
  )
}

function parseSqliteLite(bytes: Uint8Array): {
  pageSize: number
  pageCount: number
  encoding: string
  tables: string[]
  error?: string
} {
  if (bytes.byteLength < 100) {
    return { pageSize: 0, pageCount: 0, encoding: '?', tables: [], error: 'file too small' }
  }
  const magic = String.fromCharCode(...bytes.subarray(0, 16))
  if (!magic.startsWith('SQLite format 3')) {
    return {
      pageSize: 0,
      pageCount: 0,
      encoding: '?',
      tables: [],
      error: 'not a SQLite 3 database',
    }
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  let pageSize = view.getUint16(16, false)
  if (pageSize === 1) pageSize = 65536
  const pageCount = view.getUint32(28, false)
  const encCode = view.getUint32(56, false)
  const encoding = encCode === 1 ? 'UTF-8' : encCode === 2 ? 'UTF-16le' : encCode === 3 ? 'UTF-16be' : `code ${encCode}`

  // Scan printable ASCII for CREATE TABLE names (best-effort, no btree walk).
  const text = extractAscii(bytes, 256_000)
  const tables = new Set<string>()
  const re = /CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?["'`]?([A-Za-z_][\w.]*)/gi
  let m: RegExpExecArray | null
  while ((m = re.exec(text)) !== null) {
    const name = m[1]
    if (name && !name.startsWith('sqlite_')) tables.add(name)
    if (tables.size >= 200) break
  }
  return {
    pageSize,
    pageCount,
    encoding,
    tables: Array.from(tables).sort(),
  }
}

function extractAscii(bytes: Uint8Array, max: number): string {
  const n = Math.min(bytes.byteLength, max)
  let out = ''
  for (let i = 0; i < n; i++) {
    const c = bytes[i]
    if (c === 9 || c === 10 || c === 13 || (c >= 32 && c < 127)) out += String.fromCharCode(c)
    else out += ' '
  }
  return out
}

// ─── DOCX (word/document.xml → plain text) ───────────────────────────────────

function DocxBody({ bytes }: { bytes: Uint8Array }) {
  const text = useMemo(() => {
    try {
      return extractDocxText(bytes)
    } catch (e) {
      return `Failed to parse docx: ${e instanceof Error ? e.message : String(e)}`
    }
  }, [bytes])
  return (
    <pre className="whitespace-pre-wrap break-words p-4 font-mono text-[12.5px] leading-relaxed text-koma-fg">
      {text || '(empty document)'}
    </pre>
  )
}

function extractDocxText(bytes: Uint8Array): string {
  const files = unzipSync(bytes)
  const xmlBytes = files['word/document.xml']
  if (!xmlBytes) throw new Error('word/document.xml missing')
  const xml = strFromU8(xmlBytes)
  // Prefer paragraph boundaries via </w:p>
  const parts: string[] = []
  const paras = xml.split(/<\/w:p>/i)
  for (const p of paras) {
    const runs: string[] = []
    const re = /<w:t(?:\s[^>]*)?>([^<]*)<\/w:t>/g
    let m: RegExpExecArray | null
    while ((m = re.exec(p)) !== null) {
      runs.push(decodeXml(m[1]))
    }
    if (runs.length) parts.push(runs.join(''))
  }
  return parts.join('\n').trim()
}

function decodeXml(s: string): string {
  return s
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&')
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
}

// ─── Excel (sharedStrings + first sheet rows, simple) ────────────────────────

function ExcelBody({ bytes }: { bytes: Uint8Array }) {
  const grid = useMemo(() => {
    try {
      return extractExcelSheet(bytes)
    } catch (e) {
      return {
        name: 'error',
        rows: [[e instanceof Error ? e.message : String(e)]],
      }
    }
  }, [bytes])

  return (
    <div className="p-2">
      <div className="mb-2 text-[11px] text-koma-dim">Sheet: {grid.name}</div>
      <div className="overflow-auto rounded border border-koma-border">
        <table className="min-w-full border-collapse text-left text-[11.5px]">
          <tbody>
            {grid.rows.slice(0, 200).map((row, ri) => (
              <tr key={ri} className="border-b border-koma-border/60 odd:bg-koma-panel2">
                {row.slice(0, 40).map((cell, ci) => (
                  <td
                    key={ci}
                    className="max-w-[220px] truncate border-r border-koma-border/40 px-2 py-1 font-mono text-koma-fg"
                    title={cell}
                  >
                    {cell}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {grid.rows.length > 200 ? (
        <div className="mt-2 text-[11px] text-koma-dim">Showing first 200 rows.</div>
      ) : null}
    </div>
  )
}

function extractExcelSheet(bytes: Uint8Array): { name: string; rows: string[][] } {
  const files = unzipSync(bytes)
  const shared: string[] = []
  const ss = files['xl/sharedStrings.xml']
  if (ss) {
    const xml = strFromU8(ss)
    // Each <si>…</si> may contain multiple <t>
    const siRe = /<si>([\s\S]*?)<\/si>/g
    let m: RegExpExecArray | null
    while ((m = siRe.exec(xml)) !== null) {
      const inner = m[1]
      const texts: string[] = []
      const tRe = /<t(?:\s[^>]*)?>([^<]*)<\/t>/g
      let tm: RegExpExecArray | null
      while ((tm = tRe.exec(inner)) !== null) texts.push(decodeXml(tm[1]))
      shared.push(texts.join(''))
    }
  }

  // First worksheet path
  let sheetPath = 'xl/worksheets/sheet1.xml'
  let sheetName = 'Sheet1'
  const wb = files['xl/workbook.xml']
  const rels = files['xl/_rels/workbook.xml.rels']
  if (wb && rels) {
    const wbXml = strFromU8(wb)
    const relXml = strFromU8(rels)
    const sheetMatch = /<sheet[^>]*name="([^"]+)"[^>]*r:id="([^"]+)"/i.exec(wbXml)
    if (sheetMatch) {
      sheetName = decodeXml(sheetMatch[1])
      const rid = sheetMatch[2]
      const relRe = new RegExp(
        `<Relationship[^>]*Id="${rid}"[^>]*Target="([^"]+)"`,
        'i',
      )
      const rm = relRe.exec(relXml)
      if (rm) {
        const target = rm[1].replace(/^\//, '')
        sheetPath = target.startsWith('xl/') ? target : `xl/${target}`
      }
    }
  }
  const sheetBytes = files[sheetPath]
  if (!sheetBytes) throw new Error(`worksheet missing: ${sheetPath}`)
  const sheetXml = strFromU8(sheetBytes)

  const rows: string[][] = []
  const rowRe = /<row\b[^>]*>([\s\S]*?)<\/row>/g
  let rm: RegExpExecArray | null
  while ((rm = rowRe.exec(sheetXml)) !== null) {
    const rowXml = rm[1]
    const cells: { ref: string; v: string }[] = []
    const cRe = /<c\b([^>]*)>([\s\S]*?)<\/c>|<c\b([^>]*)\/>/g
    let cm: RegExpExecArray | null
    while ((cm = cRe.exec(rowXml)) !== null) {
      const attrs = cm[1] ?? cm[3] ?? ''
      const body = cm[2] ?? ''
      const refM = /\br="([A-Z]+)(\d+)"/i.exec(attrs)
      const ref = refM ? refM[1].toUpperCase() : ''
      const t = /\bt="([^"]+)"/.exec(attrs)?.[1]
      let val = ''
      const vM = /<v>([^<]*)<\/v>/.exec(body)
      const raw = vM ? vM[1] : ''
      if (t === 's' && raw !== '') {
        const idx = parseInt(raw, 10)
        val = Number.isFinite(idx) ? (shared[idx] ?? raw) : raw
      } else if (t === 'inlineStr') {
        const tM = /<t(?:\s[^>]*)?>([^<]*)<\/t>/.exec(body)
        val = tM ? decodeXml(tM[1]) : ''
      } else {
        val = decodeXml(raw)
      }
      cells.push({ ref, v: val })
    }
    // Expand by column letter order
    if (cells.length === 0) {
      rows.push([])
      continue
    }
    const withIdx = cells.map((c) => ({
      col: colLettersToIndex(c.ref || 'A'),
      v: c.v,
    }))
    const maxCol = Math.max(...withIdx.map((c) => c.col))
    const line = Array.from({ length: maxCol + 1 }, () => '')
    for (const c of withIdx) line[c.col] = c.v
    rows.push(line)
    if (rows.length >= 250) break
  }
  return { name: sheetName, rows }
}

function colLettersToIndex(letters: string): number {
  let n = 0
  for (let i = 0; i < letters.length; i++) {
    const c = letters.charCodeAt(i)
    if (c < 65 || c > 90) continue
    n = n * 26 + (c - 64)
  }
  return Math.max(0, n - 1)
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  return `${(n / (1024 * 1024)).toFixed(1)} MB`
}
