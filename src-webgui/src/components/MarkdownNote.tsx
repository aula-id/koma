import { showToast } from '../lib/toast'
import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react'
import { Bold, Code, Heading2, ImagePlus, Italic, Link, List, ListOrdered, Quote, X } from 'lucide-react'
import { LexicalMarkdownEditor, type LexicalEditorHandle } from './lexical/LexicalMarkdownEditor'
import { useKoma } from '../store/koma'
import { bytesToObjectUrl, requestFileBytes } from '../lib/filePreview'
import {
  inlineFromHtml,
  inlineToHtml,
  isNoteText,
  noteImageFile,
  normalizeDiagramNoteMarkdown,
  parseNote,
  safeNoteUrl,
  serializeNote,
  type NoteBlock,
} from '../lib/markdownNote'
import { prepareDiagramNoteMarkdown } from '../lib/diagramNoteAttach'
import { writeWorkspaceBytes } from '../lib/diagramNotes'
import { mimeForPath } from '../lib/viewerKind'

const IMAGE_LIMIT = 8 * 1024 * 1024

function toast(text: string) { showToast(text, 'error') }

function formatInline(command: string, value?: string) {
  document.execCommand(command, false, value)
  const el = document.activeElement
  if (el instanceof HTMLElement && el.isContentEditable) el.dispatchEvent(new Event('input', { bubbles: true }))
}

function NoteImage({ root, path, alt, preview }: { root: string; path: string; alt: string; preview?: string }) {
  const req = useKoma((s) => s.req)
  const [url, setUrl] = useState<string | null>(preview ?? null)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    if (preview) {
      setUrl(preview)
      return
    }
    let cancelled = false
    let objectUrl: string | null = null
    setUrl(null)
    setFailed(false)
    void requestFileBytes(req, root, path)
      .then((bytes) => {
        if (cancelled) return
        objectUrl = bytesToObjectUrl(bytes, mimeForPath(path))
        setUrl(objectUrl)
      })
      .catch(() => {
        if (!cancelled) setFailed(true)
      })
    return () => {
      cancelled = true
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [root, path, req, preview])
  if (failed) return <span className="text-[11px] text-koma-dim">{alt || path}</span>
  if (!url) return <span className="text-[11px] text-koma-dim">Loading image…</span>
  return <img src={url} alt={alt} className="max-w-full rounded" />
}

export function InlineMarkdownInput({
  text,
  className,
  style,
  'aria-label': ariaLabel,
  editorRef,
  onChange,
  onFocus,
  onBlur,
  onKeyDown,
}: {
  text: string
  className: string
  style?: React.CSSProperties
  'aria-label'?: string
  editorRef?: (el: HTMLElement | null) => void
  onChange: (text: string) => void
  onFocus?: () => void
  onBlur?: () => void
  onKeyDown?: (event: ReactKeyboardEvent<HTMLElement>) => void
}) {
  return (
    <LexicalMarkdownEditor
      profile="inline"
      markdown={text}
      onMarkdown={onChange}
      className={className}
      style={style}
      ariaLabel={ariaLabel}
      controlled
      editorRef={editorRef}
      onFocus={onFocus}
      onBlur={onBlur}
      onKeyDown={onKeyDown}
    />
  )
}

function CodeEditor({
  text,
  editorRef,
  onChange,
  onFocus,
  onKeyDown,
}: {
  text: string
  editorRef: (el: HTMLElement | null) => void
  onChange: (text: string) => void
  onFocus: () => void
  onKeyDown: (event: ReactKeyboardEvent<HTMLElement>) => void
}) {
  const ref = useRef<HTMLElement>(null)
  const focused = useRef(false)
  useEffect(() => {
    const el = ref.current
    if (!el || focused.current) return
    if ((el.textContent ?? '') !== text) el.textContent = text
  }, [text])
  return (
    <pre className="my-1 overflow-x-auto rounded bg-koma-bg px-2 py-1">
      <code
        ref={(el) => {
          ref.current = el
          editorRef(el)
        }}
        contentEditable
        suppressContentEditableWarning
        spellCheck={false}
        className="block min-h-[1.2em] whitespace-pre-wrap font-mono text-[11px] outline-none"
        onFocus={() => {
          focused.current = true
          onFocus()
        }}
        onBlur={() => {
          focused.current = false
        }}
        onInput={() => onChange(ref.current?.textContent ?? '')}
        onKeyDown={onKeyDown}
      />
    </pre>
  )
}

function noteBlocks(markdown: string): NoteBlock[] {
  const parsed = parseNote(markdown)
  return parsed.length ? parsed : [{ kind: 'p', text: '' }]
}

function imageExt(file: File): string | null {
  if (file.type === 'image/png') return 'png'
  if (file.type === 'image/jpeg') return 'jpg'
  if (file.type === 'image/gif') return 'gif'
  if (file.type === 'image/webp') return 'webp'
  return null
}

export function MarkdownNote({
  value,
  root,
  assetDir,
  legacyAssetDir,
  onChange,
  onDone,
}: {
  value: string
  root: string
  assetDir: string
  legacyAssetDir?: string
  onChange: (markdown: string) => void
  onDone: () => void
}) {
  const req = useKoma((s) => s.req)
  const hostId = useKoma((s) => s.remoteState.hostId ?? 'local')
  const fileRef = useRef<HTMLInputElement>(null)
  const [editorMarkdown, setEditorMarkdown] = useState(value)
  const sentRef = useRef(value)
  const apiRef = useRef<LexicalEditorHandle | null>(null)
  const localRef = useRef<Record<string, string>>({})
  const [link, setLink] = useState<string | null>(null)
  const [local, setLocal] = useState<Record<string, string>>({})
  localRef.current = local

  useEffect(() => {
    return () => {
      for (const url of Object.values(localRef.current)) URL.revokeObjectURL(url)
    }
  }, [])

  useEffect(() => {
    if (value === sentRef.current) {
      setEditorMarkdown(value)
      return
    }
    let cancelled = false
    const needsRepair = /\[Image #\d+\]/.test(value) || /(?<!!)\[[^\]]*\]\([A-Za-z0-9._-]+\.(?:png|jpe?g|gif|webp)\)/i.test(value)
    if (!needsRepair) {
      const md = normalizeDiagramNoteMarkdown(value)
      sentRef.current = md
      setEditorMarkdown(md)
      return
    }
    void prepareDiagramNoteMarkdown({ hostId, root }, assetDir, value, legacyAssetDir ?? null).then((md) => {
      if (cancelled) return
      sentRef.current = md
      setEditorMarkdown(md)
    })
    return () => {
      cancelled = true
    }
  }, [assetDir, hostId, legacyAssetDir, root, value])

  const upload = async (file: File) => {
    const ext = imageExt(file)
    if (!ext) {
      toast('Use a PNG, JPEG, GIF, or WebP')
      return
    }
    if (file.size > IMAGE_LIMIT) {
      toast('Image is larger than 8 MB')
      return
    }
    const name = `img-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}.${ext}`
    const bytes = new Uint8Array(await file.arrayBuffer())
    writeWorkspaceBytes(req, root, `${assetDir}/${name}`, bytes)
    const url = URL.createObjectURL(file)
    setLocal((prev) => ({ ...prev, [name]: url }))
    apiRef.current?.insertImage('image', name)
    apiRef.current?.focus()
  }

  const applyLink = () => {
    const href = link?.trim() ?? ''
    setLink(null)
    if (!href || !safeNoteUrl(href)) return
    apiRef.current?.insertLink(href)
    apiRef.current?.focus()
  }

  const tool = (label: string, icon: ReactNode, onClick: () => void) => (
    <button
      type="button"
      title={label}
      aria-label={label}
      onMouseDown={(event) => event.preventDefault()}
      onClick={onClick}
      className="flex h-6 w-6 flex-none items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
    >
      {icon}
    </button>
  )

  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      onBlur={(event) => {
        const next = event.relatedTarget
        if (next instanceof Node && event.currentTarget.contains(next)) return
        const canonical = normalizeDiagramNoteMarkdown(sentRef.current)
        if (canonical !== sentRef.current) {
          sentRef.current = canonical
          onChange(canonical)
        }
        onDone()
      }}
    >
      <div className="flex flex-none flex-wrap items-center gap-0.5 border-b border-koma-border px-2 py-1">
        {tool('Bold', <Bold size={13} />, () => apiRef.current?.format('bold'))}
        {tool('Italic', <Italic size={13} />, () => apiRef.current?.format('italic'))}
        {tool('Heading', <Heading2 size={13} />, () => apiRef.current?.toggleHeading())}
        {tool('Bullet list', <List size={13} />, () => apiRef.current?.toggleBullet())}
        {tool('Numbered list', <ListOrdered size={13} />, () => apiRef.current?.toggleNumber())}
        {tool('Quote', <Quote size={13} />, () => apiRef.current?.toggleQuote())}
        {tool('Code', <Code size={13} />, () => apiRef.current?.toggleCodeBlock())}
        {tool('Link', <Link size={13} />, () => setLink((current) => (current == null ? 'https://' : null)))}
        {tool('Image', <ImagePlus size={13} />, () => fileRef.current?.click())}
        <input
          ref={fileRef}
          type="file"
          accept="image/png,image/jpeg,image/gif,image/webp"
          className="hidden"
          onChange={(event) => {
            const file = event.target.files?.[0]
            event.target.value = ''
            if (file) void upload(file)
          }}
        />
      </div>
      {link != null ? (
        <form
          className="flex flex-none gap-1 border-b border-koma-border px-2 py-1"
          onSubmit={(event) => {
            event.preventDefault()
            applyLink()
          }}
        >
          <input
            autoFocus
            value={link}
            aria-label="Link address"
            onChange={(event) => setLink(event.target.value)}
            className="h-6 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1.5 text-[11px] text-koma-fg outline-none"
          />
        </form>
      ) : null}
      <LexicalMarkdownEditor
        profile="note"
        markdown={editorMarkdown}
        onMarkdown={(markdown) => {
          sentRef.current = markdown
          onChange(markdown)
        }}
        controlled
        placeholder="What this is for"
        apiRef={apiRef}
        noteAssets={{ root, assetDir, legacyAssetDir, previews: local }}
        className="min-h-0 flex-1 overflow-y-auto px-3 py-2 text-[12px] text-koma-fg"
        onPaste={(event) => {
          const file = [...event.clipboardData.files].find((item) => item.type.startsWith('image/'))
          if (!file) return false
          event.preventDefault()
          void upload(file)
          return true
        }}
      />
    </div>
  )
}
