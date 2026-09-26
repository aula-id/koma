import { useEffect, useMemo, useState } from 'react'
import { Streamdown, type Components } from 'streamdown'
import { useKoma, type Tab } from '../store/koma'
import { fileKey } from '../store/coding'
import { bytesToObjectUrl, requestFileBytes } from '../lib/filePreview'
import { isMarkdownPath, markdownLocalPath } from '../lib/markdownPreview'
import { mimeForPath } from '../lib/viewerKind'
import { luminance } from '../lib/luminance'
import { BrailleSpinner } from './BrailleSpinner'
import { EditorChrome } from './EditorChrome'
import { komaCode } from './komaShiki'

type CodingTab = Extract<Tab, { kind: 'codingFile' }>

function WorkspaceImage({ root, path, alt, title }: {
  root: string
  path: string
  alt?: string
  title?: string
}) {
  const req = useKoma((s) => s.req)
  const [url, setUrl] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    let cancelled = false
    let objectUrl: string | null = null
    setUrl(null)
    setFailed(false)
    void requestFileBytes(req, root, path).then((bytes) => {
      if (cancelled) return
      objectUrl = bytesToObjectUrl(bytes, mimeForPath(path))
      setUrl(objectUrl)
    }).catch(() => {
      if (!cancelled) setFailed(true)
    })
    return () => {
      cancelled = true
      if (objectUrl) URL.revokeObjectURL(objectUrl)
    }
  }, [root, path, req])
  if (failed) return <span className="text-koma-dim" title={path}>{alt || path} (image unavailable)</span>
  if (!url) return <span className="text-koma-dim">{alt || 'Loading image…'}</span>
  return <img src={url} alt={alt ?? ''} title={title} className="max-w-full" onError={() => setFailed(true)} />
}

export default function MarkdownPreviewTab({ tab }: { tab: CodingTab }) {
  const file = useKoma((s) => s.coding.files[fileKey(tab.root, tab.path)])
  const openCodingFile = useKoma((s) => s.openCodingFile)
  const bg = useKoma((s) => s.palette.bg)
  const theme = luminance(bg) >= 0.5 ? 'github-light' : 'github-dark'
  const components = useMemo<Components>(() => ({
    a: ({ href, children, title }) => {
      const localPath = href ? markdownLocalPath(tab.path, href) : null
      return (
        <a
          href={href}
          title={title}
          target={localPath || href?.startsWith('#') ? undefined : '_blank'}
          rel="noopener noreferrer"
          onClick={(event) => {
            if (localPath) {
              event.preventDefault()
              openCodingFile(tab.root, localPath, { preview: isMarkdownPath(localPath) })
            } else if (href && !/^(https?:|mailto:|tel:|\/\/)/i.test(href)) {
              // Never let workspace-relative or fragment URLs navigate the app.
              event.preventDefault()
              if (href.startsWith('#')) {
                let id: string
                try { id = decodeURIComponent(href.slice(1)) } catch { return }
                const host = event.currentTarget.closest('[data-markdown-preview]')
                const target = Array.from(host?.querySelectorAll('[id]') ?? []).find((el) => el.id === id)
                target?.scrollIntoView({ block: 'start' })
              }
            }
          }}
        >{children}</a>
      )
    },
    img: ({ src, alt, title }) => {
      const path = typeof src === 'string' ? markdownLocalPath(tab.path, src) : null
      if (path) return <WorkspaceImage root={tab.root} path={path} alt={alt} title={title} />
      if (typeof src !== 'string' || !/^(https?:|\/\/)/i.test(src)) return <span className="text-koma-dim">{alt}</span>
      return <img src={src} alt={alt ?? ''} title={title} loading="lazy" className="max-w-full" />
    },
  }), [tab.root, tab.path, openCodingFile])

  const unavailable = file?.binary ? 'Binary file — no preview'
    : file?.tooLarge ? 'File too large to preview'
    : !isMarkdownPath(tab.path) ? 'Preview is available for Markdown files'
    : file?.content == null && file?.error ? file.error : null
  const status = file?.conflict ? 'Read-only · conflict'
    : file?.dirty ? 'Read-only · unsaved changes' : 'Read-only'

  return (
    <div className="flex h-full w-full flex-col" data-markdown-preview>
      <EditorChrome
        path={tab.path}
        status={status}
        preview
        onTogglePreview={() => openCodingFile(tab.root, tab.path)}
      />
      {unavailable ? (
        <div className="flex min-h-0 flex-1 items-center justify-center px-6 text-center text-[12px] text-koma-dim">{unavailable}</div>
      ) : file?.content == null ? (
        <div className="flex min-h-0 flex-1 items-center justify-center text-koma-dim"><BrailleSpinner size={14} /></div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto p-4">
          <Streamdown
            className="koma-md"
            mode="static"
            parseIncompleteMarkdown={false}
            components={components}
            plugins={{ code: komaCode }}
            shikiTheme={[theme, theme]}
            lineNumbers={false}
            controls={{ code: { copy: true, download: false }, table: false, mermaid: false }}
          >{file.content}</Streamdown>
        </div>
      )}
    </div>
  )
}
