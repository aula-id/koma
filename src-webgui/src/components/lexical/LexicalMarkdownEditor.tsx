import { useEffect, useMemo, useRef, useState, type CSSProperties, type ClipboardEvent, type KeyboardEvent as ReactKeyboardEvent, type JSX } from 'react'
import { LexicalComposer } from '@lexical/react/LexicalComposer'
import { ContentEditable } from '@lexical/react/LexicalContentEditable'
import { HistoryPlugin } from '@lexical/react/LexicalHistoryPlugin'
import { LinkPlugin } from '@lexical/react/LexicalLinkPlugin'
import { ListPlugin } from '@lexical/react/LexicalListPlugin'
import { LexicalErrorBoundary } from '@lexical/react/LexicalErrorBoundary'
import { RichTextPlugin } from '@lexical/react/LexicalRichTextPlugin'
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { OnChangePlugin } from '@lexical/react/LexicalOnChangePlugin'
import { MarkdownShortcutPlugin } from '@lexical/react/LexicalMarkdownShortcutPlugin'
import { $createCodeNode, CodeNode } from '@lexical/code'
import { LinkNode, TOGGLE_LINK_COMMAND } from '@lexical/link'
import { INSERT_ORDERED_LIST_COMMAND, INSERT_UNORDERED_LIST_COMMAND, ListItemNode, ListNode } from '@lexical/list'
import {
  $convertFromMarkdownString,
  BOLD_STAR,
  BOLD_UNDERSCORE,
  CODE,
  HEADING,
  INLINE_CODE,
  ITALIC_STAR,
  ITALIC_UNDERSCORE,
  LINK,
  ORDERED_LIST,
  QUOTE,
  STRIKETHROUGH,
  UNORDERED_LIST,
  type TextMatchTransformer,
  type Transformer,
} from '@lexical/markdown'
import { $createHeadingNode, $createQuoteNode, HeadingNode, QuoteNode } from '@lexical/rich-text'
import { $setBlocksType } from '@lexical/selection'
import {
  $createParagraphNode,
  $createTextNode,
  $getNearestNodeFromDOMNode,
  $getNodeByKey,
  $getRoot,
  $getSelection,
  $isRangeSelection,
  $isTextNode,
  COMMAND_PRIORITY_CRITICAL,
  COMMAND_PRIORITY_HIGH,
  DRAGOVER_COMMAND,
  DROP_COMMAND,
  FORMAT_TEXT_COMMAND,
  KEY_ENTER_COMMAND,
  PASTE_COMMAND,
  type LexicalEditor,
} from 'lexical'
import { $createComposerChipNode, $isComposerChipNode, COMPOSER_CHIP_MIME, ComposerChipNode } from './chipNodes'
import { $exportLexicalMarkdown } from './lexicalMarkdown'
import { $createNoteImageNode, $isNoteImageNode, NoteAssetsContext, NoteImageNode, type NoteAssets } from './noteImageNode'

export type LexicalProfile = 'composer' | 'inline' | 'note'

export type LexicalEditorHandle = {
  focus: () => void
  insertText: (text: string) => void
  appendText: (text: string) => void
  setMarkdown: (markdown: string, edge?: 'start' | 'end') => void
  getMarkdown: () => string
  format: (kind: 'bold' | 'italic' | 'code') => void
  toggleHeading: () => void
  toggleBullet: () => void
  toggleNumber: () => void
  toggleQuote: () => void
  toggleCodeBlock: () => void
  insertLink: (url: string) => void
  insertImage: (alt: string, src: string) => void
  isAtStart: () => boolean
  isAtEnd: () => boolean
}

const MARKER = /\[(?:Image|Pasted Text) #\d+\]/
const FILE_SENTINEL = /@\[\d+\]\S+/

const markerTransformer: TextMatchTransformer = {
  dependencies: [ComposerChipNode],
  export: (node) => ($isComposerChipNode(node) && node.getTextContent().startsWith('[') ? node.getTextContent() : null),
  importRegExp: MARKER,
  regExp: /\[(?:Image|Pasted Text) #\d+\]$/,
  replace: (node, match) => {
    node.replace($createComposerChipNode(match[0], 'attach'))
  },
  trigger: ']',
  type: 'text-match',
}

const fileTransformer: TextMatchTransformer = {
  dependencies: [ComposerChipNode],
  export: (node) => ($isComposerChipNode(node) && node.getTextContent().startsWith('@') ? node.getTextContent() : null),
  importRegExp: FILE_SENTINEL,
  regExp: /@\[\d+\]\S+$/,
  replace: (node, match) => {
    node.replace($createComposerChipNode(match[0], 'file'))
  },
  trigger: ' ',
  type: 'text-match',
}

const imageTransformer: TextMatchTransformer = {
  dependencies: [NoteImageNode],
  export: (node) => ($isNoteImageNode(node) ? node.getTextContent() : null),
  importRegExp: /!\[([^\]]*)\]\(([^)\s]+)\)/,
  regExp: /!\[([^\]]*)\]\(([^)\s]+)\)$/,
  replace: (node, match) => {
    node.replace($createNoteImageNode(match[1] ?? '', match[2] ?? ''))
  },
  trigger: ')',
  type: 'text-match',
}

const INLINE: Transformer[] = [INLINE_CODE, BOLD_STAR, BOLD_UNDERSCORE, ITALIC_STAR, ITALIC_UNDERSCORE, STRIKETHROUGH, LINK]
const COMPOSER: Transformer[] = [CODE, ...INLINE, markerTransformer, fileTransformer]
const NOTE: Transformer[] = [CODE, HEADING, QUOTE, UNORDERED_LIST, ORDERED_LIST, ...INLINE, imageTransformer]

function transformersFor(profile: LexicalProfile): Transformer[] {
  if (profile === 'note') return NOTE
  if (profile === 'composer') return COMPOSER
  return INLINE
}


function namedImage(file: File): File {
  if (file.name) return file
  const ext = file.type.split('/')[1]?.replace('jpeg', 'jpg') || 'png'
  return new File([file], `clipboard.${ext}`, { type: file.type || 'image/png' })
}

function imageFilesFrom(data: DataTransfer | null): File[] {
  if (!data) return []
  const files: File[] = []
  for (const item of Array.from(data.items ?? [])) {
    if (!item.type.startsWith('image/')) continue
    const file = item.getAsFile()
    if (file) files.push(namedImage(file))
  }
  if (files.length) return files
  for (const file of Array.from(data.files ?? [])) {
    if (file.type.startsWith('image/')) files.push(namedImage(file))
  }
  return files
}

async function readClipboardImages(): Promise<File[]> {
  if (!navigator.clipboard?.read) return []
  try {
    const items = await navigator.clipboard.read()
    const files: File[] = []
    for (const item of items) {
      const type = item.types.find((entry) => entry.startsWith('image/'))
      if (!type) continue
      const blob = await item.getType(type)
      const ext = type.split('/')[1]?.replace('jpeg', 'jpg') || 'png'
      files.push(new File([blob], `clipboard.${ext}`, { type }))
    }
    return files
  } catch {
    return []
  }
}

function $chipKnownTokens(tokens: readonly string[]) {
  const sorted = [...tokens].filter((token) => token.length > 0).sort((a, b) => b.length - a.length)
  if (!sorted.length) return
  let guard = 0
  while (guard < 40) {
    guard += 1
    let replaced = false
    for (const textNode of $getRoot().getAllTextNodes()) {
      const text = textNode.getTextContent()
      const token = sorted.find((item) => text.includes(item))
      if (!token) continue
      const index = text.indexOf(token)
      const parts = textNode.splitText(index, index + token.length)
      const target = parts.find((part) => part.getTextContent() === token)
      if (!target) continue
      target.replace($createComposerChipNode(token, 'file'))
      replaced = true
      break
    }
    if (!replaced) break
  }
}

function $prepareInsert(atEnd: boolean) {
  const root = $getRoot()
  if (root.getChildrenSize() === 0) {
    const paragraph = $createParagraphNode()
    root.append(paragraph)
    paragraph.select()
    return
  }
  if (atEnd || !$isRangeSelection($getSelection())) root.selectEnd()
}

function $insertPiece(text: string, tokens: readonly string[], atEnd = false) {
  $prepareInsert(atEnd)
  const active = $getSelection()
  if (!$isRangeSelection(active)) return
  const trimmed = text.trim()
  const marker = trimmed.match(/^\[(?:Image|Pasted Text) #\d+\]$/)
  if (marker) {
    active.insertNodes([$createComposerChipNode(trimmed, 'attach')])
    if (text.endsWith(' ')) active.insertNodes([$createTextNode(' ')])
    return
  }
  const token = [...tokens].sort((a, b) => b.length - a.length).find((item) => item.length > 0 && text.includes(item))
  if (token && text.trim() === token) {
    active.insertNodes([$createComposerChipNode(token, 'file')])
    if (text.endsWith(' ')) active.insertNodes([$createTextNode(' ')])
    return
  }
  active.insertText(text)
}

function readMarkdown(editor: LexicalEditor, _transformers: Transformer[]): string {
  let markdown = ''
  editor.getEditorState().read(() => {
    markdown = $exportLexicalMarkdown()
  })
  return markdown
}

function EditorPlugins({
  profile,
  markdown,
  tokens,
  controlled,
  onMarkdown,
  onKeyDown,
  onPaste,
  onPasteFiles,
  onSubmit,
  apiRef,
  editorElementRef,
}: {
  profile: LexicalProfile
  markdown: string
  tokens: readonly string[]
  controlled: boolean
  onMarkdown: (markdown: string) => void
  onKeyDown?: (event: ReactKeyboardEvent<HTMLElement>) => void
  onPaste?: (event: ClipboardEvent) => boolean
  onPasteFiles?: (files: File[]) => void
  onSubmit?: () => void
  apiRef?: { current: LexicalEditorHandle | null }
  editorElementRef?: (el: HTMLElement | null) => void
}) {
  const [editor] = useLexicalComposerContext()
  const transformers = useMemo(() => transformersFor(profile), [profile])
  const suppress = useRef(false)
  const last = useRef<string | null>(null)
  const tokensRef = useRef(tokens)
  tokensRef.current = tokens
  const onKeyDownRef = useRef(onKeyDown)
  onKeyDownRef.current = onKeyDown
  const onPasteRef = useRef(onPaste)
  onPasteRef.current = onPaste
  const onPasteFilesRef = useRef(onPasteFiles)
  onPasteFilesRef.current = onPasteFiles
  const imagePasteAt = useRef(0)
  const onSubmitRef = useRef(onSubmit)
  onSubmitRef.current = onSubmit

  const applyMarkdown = (next: string, edge?: 'start' | 'end') => {
    suppress.current = true
    editor.update(() => {
      $convertFromMarkdownString(next, transformers, undefined, false)
      if (profile === 'composer') $chipKnownTokens(tokensRef.current)
      if (edge === 'end') $getRoot().selectEnd()
      else if (edge === 'start') $getRoot().selectStart()
    })
    last.current = next
    queueMicrotask(() => {
      suppress.current = false
    })
  }

  useEffect(() => {
    const handle: LexicalEditorHandle = {
      focus: () => editor.focus(),
      insertText: (text) => {
        editor.update(() => {
          $insertPiece(text, tokensRef.current, false)
        })
      },
      appendText: (text) => {
        editor.update(() => {
          $insertPiece(text, tokensRef.current, true)
        })
      },
      setMarkdown: applyMarkdown,
      getMarkdown: () => readMarkdown(editor, transformers),
      format: (kind) => {
        editor.focus()
        editor.dispatchCommand(FORMAT_TEXT_COMMAND, kind)
      },
      toggleHeading: () => {
        editor.update(() => {
          const selection = $getSelection()
          if (!$isRangeSelection(selection)) return
          const top = selection.anchor.getNode().getTopLevelElement()
          $setBlocksType(selection, () => (top?.getType() === 'heading' ? $createParagraphNode() : $createHeadingNode('h2')))
        })
      },
      toggleBullet: () => {
        editor.dispatchCommand(INSERT_UNORDERED_LIST_COMMAND, undefined)
      },
      toggleNumber: () => {
        editor.dispatchCommand(INSERT_ORDERED_LIST_COMMAND, undefined)
      },
      toggleQuote: () => {
        editor.update(() => {
          const selection = $getSelection()
          if (!$isRangeSelection(selection)) return
          const top = selection.anchor.getNode().getTopLevelElement()
          $setBlocksType(selection, () => (top?.getType() === 'quote' ? $createParagraphNode() : $createQuoteNode()))
        })
      },
      toggleCodeBlock: () => {
        editor.update(() => {
          const selection = $getSelection()
          if (!$isRangeSelection(selection)) return
          const top = selection.anchor.getNode().getTopLevelElement()
          $setBlocksType(selection, () => (top?.getType() === 'code' ? $createParagraphNode() : $createCodeNode()))
        })
      },
      insertLink: (url) => {
        editor.focus()
        editor.dispatchCommand(TOGGLE_LINK_COMMAND, url)
      },
      insertImage: (alt, src) => {
        editor.update(() => {
          let selection = $getSelection()
          if (!$isRangeSelection(selection)) {
            $getRoot().selectEnd()
            selection = $getSelection()
          }
          if ($isRangeSelection(selection)) selection.insertNodes([$createNoteImageNode(alt, src)])
        })
      },
      isAtStart: () => {
        let at = false
        editor.getEditorState().read(() => {
          const selection = $getSelection()
          if (!$isRangeSelection(selection) || !selection.isCollapsed()) return
          const top = selection.anchor.getNode().getTopLevelElement()
          at = top != null && top === $getRoot().getFirstChild() && selection.anchor.offset === 0
        })
        return at
      },
      isAtEnd: () => {
        let at = false
        editor.getEditorState().read(() => {
          const selection = $getSelection()
          if (!$isRangeSelection(selection) || !selection.isCollapsed()) return
          const node = selection.focus.getNode()
          const top = node.getTopLevelElement()
          const size = $isTextNode(node) ? node.getTextContentSize() : node.getChildrenSize()
          at = top != null && top === $getRoot().getLastChild() && selection.focus.offset === size
        })
        return at
      },
    }
    if (apiRef) apiRef.current = handle
    return () => {
      if (apiRef) apiRef.current = null
    }
    // applyMarkdown closes over transformers; editor identity is stable.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [apiRef, editor, profile, transformers])

  useEffect(() => {
    if (!controlled) return
    if (markdown === last.current) return
    applyMarkdown(markdown)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [controlled, markdown])

  useEffect(() => {
    return editor.registerCommand(
      KEY_ENTER_COMMAND,
      (event) => {
        if (event?.shiftKey) return false
        if (profile !== 'composer') return false
        if (event) onKeyDownRef.current?.(event as unknown as ReactKeyboardEvent<HTMLElement>)
        else onSubmitRef.current?.()
        return true
      },
      COMMAND_PRIORITY_HIGH,
    )
  }, [editor, profile])

  useEffect(() => {
    const deliverImages = (files: File[]) => {
      if (!files.length) return
      const now = performance.now()
      if (now - imagePasteAt.current < 400) return
      imagePasteAt.current = now
      onPasteFilesRef.current?.(files)
    }
    const onNativePaste = (event: ClipboardEvent) => {
      const files = imageFilesFrom(event.clipboardData)
      if (!files.length) return
      event.preventDefault()
      event.stopPropagation()
      deliverImages(files)
    }
    const root = editor.getRootElement()
    root?.addEventListener('paste', onNativePaste, true)
    const unregisterPaste = editor.registerCommand(
      PASTE_COMMAND,
      (event) => {
        const data = event && 'clipboardData' in event ? event.clipboardData : null
        const files = imageFilesFrom(data)
        if (files.length) {
          event.preventDefault()
          deliverImages(files)
          return true
        }
        const types = data ? Array.from(data.types) : []
        if (types.some((type) => type.startsWith('image/')) && !types.includes('text/plain')) {
          event.preventDefault()
          void readClipboardImages().then(deliverImages)
          return true
        }
        if (event instanceof ClipboardEvent) return onPasteRef.current?.(event) ?? false
        return false
      },
      COMMAND_PRIORITY_CRITICAL,
    )
    const unregisterOver = editor.registerCommand(
      DRAGOVER_COMMAND,
      (event) => {
        const types = event.dataTransfer ? Array.from(event.dataTransfer.types) : []
        if (!types.includes(COMPOSER_CHIP_MIME)) return false
        event.preventDefault()
        if (event.dataTransfer) event.dataTransfer.dropEffect = 'move'
        return true
      },
      COMMAND_PRIORITY_HIGH,
    )
    const unregisterDrop = editor.registerCommand(
      DROP_COMMAND,
      (event) => {
        const plain = event.dataTransfer?.getData('text/plain') ?? ''
        const mimeKey = event.dataTransfer?.getData(COMPOSER_CHIP_MIME) ?? ''
        const key =
          mimeKey && mimeKey !== 'marker'
            ? mimeKey
            : plain.startsWith('koma-chip:')
              ? plain.slice('koma-chip:'.length)
              : ''
        const marker = plain.startsWith('koma-marker:') ? plain.slice('koma-marker:'.length) : ''
        if (!marker && !key) return false
        event.preventDefault()
        const range = document.caretRangeFromPoint(event.clientX, event.clientY)
        editor.update(() => {
          const existing = key ? $getNodeByKey(key) : null
          const chip = existing && $isComposerChipNode(existing) ? existing : null
          const label = chip ? chip.getTextContent() : marker
          if (!label) return
          const tone = chip ? chip.getTone() : 'attach'
          const created = $createComposerChipNode(label, tone)
          const dom = range?.startContainer
          const el = dom instanceof Element ? dom : dom?.parentElement ?? null
          const nearest = el ? $getNearestNodeFromDOMNode(el) : null
          if ($isTextNode(nearest) && range) {
            const offset = Math.min(range.startOffset, nearest.getTextContentSize())
            nearest.select(offset, offset)
            const selection = $getSelection()
            if ($isRangeSelection(selection)) selection.insertNodes([created])
            else nearest.insertAfter(created)
          } else if (nearest && nearest.getKey() !== chip?.getKey()) {
            nearest.insertAfter(created)
          } else if (chip) {
            chip.insertAfter(created)
          } else {
            $prepareInsert(true)
            const selection = $getSelection()
            if ($isRangeSelection(selection)) selection.insertNodes([created])
          }
          if (chip && chip.getKey() !== created.getKey()) chip.remove()
        })
        return true
      },
      COMMAND_PRIORITY_HIGH,
    )
    return () => {
      root?.removeEventListener('paste', onNativePaste, true)
      unregisterPaste()
      unregisterOver()
      unregisterDrop()
    }
  }, [editor])

  return (
    <>
      <RichTextPlugin
        contentEditable={
          <ContentEditable
            ref={editorElementRef}
            aria-label={undefined}
            className="outline-none"
            onKeyDown={(event) => {
              if (event.key === 'Enter' && !event.shiftKey && profile === 'composer') return
              onKeyDown?.(event)
            }}
          />
        }
        placeholder={null}
        ErrorBoundary={LexicalErrorBoundary}
      />
      <HistoryPlugin />
      <LinkPlugin />
      {profile === 'note' ? <ListPlugin /> : null}
      <MarkdownShortcutPlugin transformers={transformers} />
      <OnChangePlugin
        onChange={() => {
          if (suppress.current) return
          const next = readMarkdown(editor, transformers)
          if (next === last.current) return
          last.current = next
          onMarkdown(next)
        }}
      />
    </>
  )
}

export function LexicalMarkdownEditor({
  profile,
  markdown,
  onMarkdown,
  className,
  style,
  placeholder,
  ariaLabel,
  tokens = [],
  controlled = false,
  onKeyDown,
  onPaste,
  onPasteFiles,
  onSubmit,
  onFocus,
  onBlur,
  apiRef,
  editorRef,
  noteAssets,
}: {
  profile: LexicalProfile
  markdown: string
  onMarkdown: (markdown: string) => void
  className?: string
  style?: CSSProperties
  placeholder?: string
  ariaLabel?: string
  tokens?: readonly string[]
  controlled?: boolean
  onKeyDown?: (event: ReactKeyboardEvent<HTMLElement>) => void
  onPaste?: (event: ClipboardEvent) => boolean
  onPasteFiles?: (files: File[]) => void
  onSubmit?: () => void
  onFocus?: () => void
  onBlur?: () => void
  apiRef?: { current: LexicalEditorHandle | null }
  editorRef?: (el: HTMLElement | null) => void
  noteAssets?: NoteAssets
}): JSX.Element {
  const nodes = useMemo(
    () => [HeadingNode, QuoteNode, ListNode, ListItemNode, LinkNode, CodeNode, ComposerChipNode, NoteImageNode],
    [],
  )
  const initial = useRef(markdown)
  const body = (
    <LexicalComposer
      initialConfig={{
        namespace: `koma-${profile}`,
        nodes,
        theme: {
          paragraph: 'm-0',
          heading: { h1: 'my-1 text-[15px] font-semibold', h2: 'my-1 text-[13px] font-semibold', h3: 'my-1 text-[12px] font-semibold' },
          quote: 'my-1 border-l-2 border-koma-dim pl-2 text-koma-dim',
          list: { ul: 'my-1 list-disc pl-4', ol: 'my-1 list-decimal pl-4', listitem: 'my-0.5' },
          text: {
            bold: 'font-semibold',
            italic: 'italic',
            strikethrough: 'line-through',
            code: 'rounded bg-koma-panel2 px-0.5 font-mono text-[0.92em]',
          },
          link: 'text-koma-accent underline',
          code: 'my-1 block overflow-x-auto rounded bg-koma-bg px-2 py-1 font-mono text-[11px]',
        },
        editorState: () => {
          $convertFromMarkdownString(initial.current, transformersFor(profile), undefined, false)
          if (profile === 'composer') $chipKnownTokens(tokens)
        },
        onError: (error) => {
          throw error
        },
      }}
    >
      <div
        className={`relative ${className ?? ''}`}
        style={style}
        aria-label={ariaLabel}
        onFocus={onFocus}
        onBlur={onBlur}
      >
        <EditorPlugins
          profile={profile}
          markdown={markdown}
          tokens={tokens}
          controlled={controlled}
          onMarkdown={onMarkdown}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
          onPasteFiles={onPasteFiles}
          onSubmit={onSubmit}
          apiRef={apiRef}
          editorElementRef={editorRef}
        />
        {placeholder ? <Placeholder text={placeholder} /> : null}
      </div>
    </LexicalComposer>
  )
  if (!noteAssets) return body
  return <NoteAssetsContext.Provider value={noteAssets}>{body}</NoteAssetsContext.Provider>
}


function Placeholder({ text }: { text: string }) {
  const [editor] = useLexicalComposerContext()
  const [empty, setEmpty] = useState(true)
  useEffect(() => {
    return editor.registerUpdateListener(({ editorState }) => {
      editorState.read(() => {
        const text = $getRoot().getTextContent()
        setEmpty(text.length === 0)
      })
    })
  }, [editor])
  if (!empty) return null
  return <div className="pointer-events-none absolute left-0 top-0 text-koma-fg/40">{text}</div>
}
