import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react'
import { Bold, Code, Heading2, ImagePlus, Italic, Link, List, ListOrdered, X } from 'lucide-react'
import { useKoma } from '../store/koma'
import { bytesToObjectUrl, requestFileBytes } from '../lib/filePreview'
import {
  inlineFromHtml,
  inlineToHtml,
  isNoteText,
  noteImageFile,
  parseNote,
  safeNoteUrl,
  serializeNote,
  type NoteBlock,
} from '../lib/markdownNote'
import { writeWorkspaceBytes } from '../lib/diagramNotes'
import { mimeForPath } from '../lib/viewerKind'

const IMAGE_LIMIT = 8 * 1024 * 1024

function toast(text: string) {
  useKoma.setState((s) => {
    const id = s.ui.toastSeq + 1
    return { ui: { ...s.ui, toastSeq: id, toast: { id, text, kind: 'error' } } }
  })
}

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
  const ref = useRef<HTMLElement>(null)
  const focused = useRef(false)
  useEffect(() => {
    const el = ref.current
    if (!el || focused.current) return
    const html = inlineToHtml(text)
    if (el.innerHTML !== (html || '<br>')) el.innerHTML = html || '<br>'
  }, [text])
  return (
    <div
      ref={(el) => {
        ref.current = el
        editorRef?.(el)
      }}
      contentEditable
      suppressContentEditableWarning
      role="textbox"
      aria-label={ariaLabel}
      aria-multiline="true"
      className={className}
      style={style}
      onFocus={() => {
        focused.current = true
        onFocus?.()
      }}
      onBlur={() => {
        focused.current = false
        onBlur?.()
      }}
      onInput={() => onChange(inlineFromHtml(ref.current?.innerHTML ?? ''))}
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
  onChange,
  onDone,
}: {
  value: string
  root: string
  assetDir: string
  onChange: (markdown: string) => void
  onDone: () => void
}) {
  const req = useKoma((s) => s.req)
  const fileRef = useRef<HTMLInputElement>(null)
  const refs = useRef<Array<HTMLElement | null>>([])
  const pending = useRef<number | null>(null)
  const savedRange = useRef<Range | null>(null)
  const localRef = useRef<Record<string, string>>({})
  const [focused, setFocused] = useState(0)
  const [link, setLink] = useState<string | null>(null)
  const [local, setLocal] = useState<Record<string, string>>({})
  const [shown, setShown] = useState<NoteBlock[]>(() => noteBlocks(value))
  const written = useRef(value)
  localRef.current = local

  useEffect(() => {
    if (value === written.current) return
    written.current = value
    setShown(noteBlocks(value))
  }, [value])

  useEffect(() => {
    if (pending.current == null) return
    const index = pending.current
    pending.current = null
    refs.current[index]?.focus()
  }, [shown])

  useEffect(() => {
    return () => {
      for (const url of Object.values(localRef.current)) URL.revokeObjectURL(url)
    }
  }, [])

  const emit = (next: NoteBlock[]) => {
    const blocks: NoteBlock[] = next.length ? next : [{ kind: 'p', text: '' }]
    setShown(blocks)
    const markdown = serializeNote(blocks)
    written.current = markdown
    if (markdown !== value) onChange(markdown)
  }

  const replace = (index: number, block: NoteBlock) => {
    emit(shown.map((item, i) => (i === index ? block : item)))
  }

  const focusBlock = (index: number) => {
    pending.current = index
    setFocused(index)
  }

  const insertAfter = (index: number, block: NoteBlock) => {
    const next = [...shown]
    next.splice(index + 1, 0, block)
    focusBlock(index + 1)
    emit(next)
  }

  const removeAt = (index: number) => {
    if (shown.length === 1) {
      replace(0, { kind: 'p', text: '' })
      return
    }
    focusBlock(Math.max(0, index - 1))
    emit(shown.filter((_, i) => i !== index))
  }

  const turnList = (kind: 'ul' | 'ol') => {
    const block = shown[focused]
    if (!block) return
    if (block.kind === kind) {
      const next = [...shown]
      next.splice(focused, 1, ...block.items.map((item) => ({ kind: 'p' as const, text: item })))
      emit(next)
      return
    }
    if (block.kind === 'p' || block.kind === 'h1' || block.kind === 'h2' || block.kind === 'h3' || block.kind === 'quote') {
      replace(focused, { kind, items: [block.text] })
    }
  }

  const onKeyDown = (index: number, text: string) => (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.key === 'Enter' && event.shiftKey) {
      event.preventDefault()
      document.execCommand('insertLineBreak')
      return
    }
    if (event.key === 'Enter') {
      event.preventDefault()
      const block = shown[index]
      if (block?.kind === 'ul' || block?.kind === 'ol') return
      insertAfter(index, { kind: 'p', text: '' })
      return
    }
    if (event.key === 'Backspace' && text === '' && shown.length > 1) {
      event.preventDefault()
      removeAt(index)
    }
  }

  const applyLink = () => {
    const href = link?.trim() ?? ''
    setLink(null)
    if (!href || !safeNoteUrl(href)) return
    refs.current[focused]?.focus()
    const sel = document.getSelection()
    if (savedRange.current && sel) {
      sel.removeAllRanges()
      sel.addRange(savedRange.current)
    }
    formatInline('createLink', href)
  }

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
    insertAfter(focused, { kind: 'image', alt: file.name.replace(/\.[^.]+$/, ''), src: name })
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
        onDone()
      }}
    >
      <div className="flex flex-none flex-wrap items-center gap-0.5 border-b border-koma-border px-2 py-1">
        {tool('Bold', <Bold size={13} />, () => formatInline('bold'))}
        {tool('Italic', <Italic size={13} />, () => formatInline('italic'))}
        {tool('Heading', <Heading2 size={13} />, () => {
          const block = shown[focused]
          if (!block || !isNoteText(block)) return
          const kind = block.kind === 'p' ? 'h2' : block.kind === 'h2' ? 'h3' : 'p'
          replace(focused, { kind, text: block.text })
        })}
        {tool('Bullet list', <List size={13} />, () => turnList('ul'))}
        {tool('Numbered list', <ListOrdered size={13} />, () => turnList('ol'))}
        {tool('Code', <Code size={13} />, () => {
          const block = shown[focused]
          if (!block) return
          if (block.kind === 'code') replace(focused, { kind: 'p', text: block.text })
          else if (isNoteText(block)) replace(focused, { kind: 'code', text: block.text })
        })}
        {tool('Link', <Link size={13} />, () => {
          const sel = document.getSelection()
          savedRange.current = sel && sel.rangeCount ? sel.getRangeAt(0).cloneRange() : null
          setLink((current) => (current == null ? 'https://' : null))
        })}
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
      <div
        className="relative min-h-0 flex-1 overflow-y-auto px-3 py-2 text-[12px] text-koma-fg [&_a]:text-koma-accent [&_a]:underline [&_code]:font-mono [&_code]:text-koma-accent [&_strong]:font-semibold"
        onPaste={(event) => {
          const file = [...event.clipboardData.files].find((item) => item.type.startsWith('image/'))
          if (!file) return
          event.preventDefault()
          void upload(file)
        }}
      >
        {shown.map((block, index) => {
          const key = `${index}-${block.kind}`
          if (block.kind === 'image') {
            const src = block.src
            const remote = noteImageFile(src) ? `${assetDir}/${src}` : ''
            return (
              <figure key={key} className="relative my-2">
                {remote ? (
                  <NoteImage root={root} path={remote} alt={block.alt} preview={local[src]} />
                ) : safeNoteUrl(src) && /^https?:\/\//i.test(src) ? (
                  <img src={src} alt={block.alt} className="max-w-full rounded" />
                ) : (
                  <span className="text-[11px] text-koma-dim">{block.alt || src}</span>
                )}
                <button
                  type="button"
                  aria-label="Remove image"
                  onClick={() => removeAt(index)}
                  className="absolute right-1 top-1 flex h-5 w-5 items-center justify-center rounded bg-koma-panel text-koma-dim hover:text-koma-fg"
                >
                  <X size={11} />
                </button>
              </figure>
            )
          }
          if (block.kind === 'code') {
            return (
              <CodeEditor
                key={key}
                text={block.text}
                editorRef={(el) => {
                  refs.current[index] = el
                }}
                onFocus={() => setFocused(index)}
                onChange={(text) => replace(index, { kind: 'code', text })}
                onKeyDown={onKeyDown(index, block.text)}
              />
            )
          }
          if (block.kind === 'ul' || block.kind === 'ol') {
            const ListTag = block.kind === 'ol' ? 'ol' : 'ul'
            return (
              <ListTag key={key} className={`my-1 pl-4 ${block.kind === 'ol' ? 'list-decimal' : 'list-disc'}`}>
                {block.items.map((item, itemIndex) => (
                  <li key={itemIndex}>
                    <InlineMarkdownInput
                      text={item}
                      className="min-h-[1.2em] outline-none"
                      editorRef={(el) => {
                        if (itemIndex === 0) refs.current[index] = el
                      }}
                      onFocus={() => setFocused(index)}
                      onChange={(text) => {
                        const items = block.items.map((entry, i) => (i === itemIndex ? text : entry))
                        replace(index, { kind: block.kind, items })
                      }}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter' && !event.shiftKey) {
                          event.preventDefault()
                          if (item === '') {
                            const items = block.items.filter((_, i) => i !== itemIndex)
                            const next = [...shown]
                            const paragraphs = [{ kind: 'p' as const, text: '' }]
                            next.splice(index, 1, ...(items.length ? [{ kind: block.kind, items }] : []), ...paragraphs)
                            focusBlock(index + (items.length ? 1 : 0))
                            emit(next)
                            return
                          }
                          const items = [...block.items]
                          items.splice(itemIndex + 1, 0, '')
                          replace(index, { kind: block.kind, items })
                        }
                      }}
                    />
                  </li>
                ))}
              </ListTag>
            )
          }
          const className =
            block.kind === 'h1'
              ? 'my-1 text-[15px] font-semibold outline-none'
              : block.kind === 'h2'
                ? 'my-1 text-[13px] font-semibold outline-none'
                : block.kind === 'h3'
                  ? 'my-1 text-[12px] font-semibold outline-none'
                  : block.kind === 'quote'
                    ? 'my-1 border-l-2 border-koma-dim pl-2 text-koma-dim outline-none'
                    : 'my-1 min-h-[1.2em] outline-none'
          if (!isNoteText(block)) return null
          return (
            <InlineMarkdownInput
              key={key}
              text={block.text}
              className={className}
              editorRef={(el) => {
                refs.current[index] = el
              }}
              onFocus={() => setFocused(index)}
              onChange={(text) => replace(index, { ...block, text })}
              onKeyDown={onKeyDown(index, block.text)}
            />
          )
        })}
        {value.trim() === '' ? <p className="pointer-events-none absolute left-3 top-2 text-koma-dim">What this is for</p> : null}
      </div>
    </div>
  )
}
