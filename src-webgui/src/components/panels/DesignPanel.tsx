import { useEffect, useState, type FormEvent, type KeyboardEvent, type MouseEvent as ReactMouseEvent } from 'react'
import { Check, File, Pencil, Trash2, X } from 'lucide-react'
import { AccordionSection } from '../AccordionSection'
import { BrailleSpinner } from '../BrailleSpinner'
import { AddBtn, Empty, IconBtn } from './helpers'
import { Select } from './form'
import { useKoma } from '../../store/koma'
import { fileKey, type FileTreeEntry } from '../../store/coding'
import {
  COMPONENT_MIME,
  DESIGN_MIME,
  addDesignToken,
  designChatText,
  designFileName,
  dropDesignToken,
  isDesignPath,
  setDesignMode,
  setDesignTokenValue,
  type DesignDoc,
  type DesignToken,
  type DesignTokenKind,
} from '../../lib/design'

const SHAPES: { kind: 'frame' | 'rect' | 'text'; label: string }[] = [
  { kind: 'frame', label: 'Frame' },
  { kind: 'rect', label: 'Rectangle' },
  { kind: 'text', label: 'Text' },
]

function ShapeTile({ kind, label }: { kind: 'frame' | 'rect' | 'text'; label: string }) {
  return (
    <div
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData(DESIGN_MIME, kind)
        e.dataTransfer.setData('text/plain', kind)
        try {
          e.dataTransfer.effectAllowed = 'copy'
        } catch {
          /* ignore */
        }
      }}
      className="flex cursor-grab items-center justify-center rounded p-0.5 text-koma-dim hover:bg-koma-hover hover:text-koma-fg active:cursor-grabbing"
      title={`Drag ${label} onto the canvas`}
    >
      {kind === 'text' ? (
        <span className="flex h-8 items-center px-1 text-[13px] leading-none text-koma-fg/80">Text</span>
      ) : (
        <span className={`flex h-8 items-center justify-center rounded border border-current bg-koma-bg text-[11px] leading-none text-koma-fg/80 ${kind === 'frame' ? 'w-16' : 'w-10'}`}>
          {kind === 'frame' ? 'Frame' : 'Rect'}
        </span>
      )}
    </div>
  )
}

const EMPTY_ROOTS: string[] = []
const TOKEN_KINDS: { kind: DesignTokenKind; label: string }[] = [
  { kind: 'color', label: 'Color' },
  { kind: 'space', label: 'Space' },
  { kind: 'type', label: 'Type' },
  { kind: 'radius', label: 'Radius' },
]

function commitDesign(root: string, path: string, doc: DesignDoc, update: (root: string, path: string, doc: DesignDoc) => void) {
  const event = new CustomEvent('koma-design-commit', { cancelable: true, detail: { root, path, doc } })
  if (!window.dispatchEvent(event)) return
  update(root, path, doc)
}

function TokenValue({ token, mode, onValue }: { token: DesignToken; mode: string; onValue: (value: string) => void }) {
  const value = token.values[mode] ?? ''
  if (token.kind === 'color') {
    return (
      <input
        type="color"
        aria-label={`${token.name} ${mode}`}
        value={value.startsWith('#') ? value : '#1a1d27'}
        onChange={(event) => onValue(event.target.value.toLowerCase())}
        className="h-5 w-7 flex-none cursor-pointer rounded border border-koma-border bg-transparent"
      />
    )
  }
  if (token.kind === 'type') {
    const [size, weight] = value.split('/')
    return (
      <span className="flex flex-none items-center gap-1">
        <input
          type="number"
          min={1}
          aria-label={`${token.name} size`}
          value={size || '13'}
          onChange={(event) => {
            if (!/^\d+(\.\d+)?$/.test(event.target.value)) return
            onValue(`${event.target.value}/${weight || 'regular'}`)
          }}
          className="h-5 w-12 rounded border border-koma-border bg-koma-bg px-1 text-[12px] text-koma-fg outline-none"
        />
        <select
          aria-label={`${token.name} weight`}
          value={weight || 'regular'}
          onChange={(event) => onValue(`${size || '13'}/${event.target.value}`)}
          className="h-5 rounded border border-koma-border bg-koma-bg text-[12px] text-koma-fg outline-none"
        >
          <option value="regular">Regular</option>
          <option value="medium">Medium</option>
          <option value="bold">Bold</option>
        </select>
      </span>
    )
  }
  return (
    <input
      type="number"
      min={0}
      aria-label={`${token.name} ${mode}`}
      value={value}
      onChange={(event) => {
        if (!/^\d+(\.\d+)?$/.test(event.target.value)) return
        onValue(event.target.value)
      }}
      className="h-5 w-14 flex-none rounded border border-koma-border bg-koma-bg px-1 text-[12px] text-koma-fg outline-none"
    />
  )
}

function TokenEditor({ root, path, doc, onCommit }: { root: string; path: string; doc: DesignDoc; onCommit: (doc: DesignDoc) => void }) {
  const [name, setName] = useState('')
  const [kind, setKind] = useState<DesignTokenKind>('color')
  const [error, setError] = useState<string | null>(null)
  const add = () => {
    const next = addDesignToken(doc, name, kind)
    if (!next) {
      setError('Use a new name like color.fg')
      return
    }
    setError(null)
    setName('')
    onCommit(next)
  }
  return (
    <div className="flex flex-col gap-1 px-2 py-1.5 text-[12px]">
      <div className="flex flex-wrap gap-1">
        {doc.modes.map((mode) => (
          <button
            key={mode}
            type="button"
            aria-pressed={doc.mode === mode}
            onClick={() => onCommit(setDesignMode(doc, mode))}
            className={`h-6 rounded px-1.5 capitalize ${doc.mode === mode ? 'bg-koma-accent/20 text-koma-accent' : 'text-koma-dim hover:bg-koma-hover'}`}
          >
            {mode}
          </button>
        ))}
      </div>
      <form
        className="flex items-center gap-1"
        onSubmit={(event) => {
          event.preventDefault()
          add()
        }}
      >
        <input
          value={name}
          placeholder="color.fg"
          aria-label="Token name"
          onChange={(event) => setName(event.target.value)}
          className="h-6 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1.5 text-[12px] text-koma-fg outline-none"
        />
        <select
          aria-label="Token kind"
          value={kind}
          onChange={(event) => setKind(event.target.value as DesignTokenKind)}
          className="h-6 rounded border border-koma-border bg-koma-bg text-[12px] text-koma-fg outline-none"
        >
          {TOKEN_KINDS.map((item) => (
            <option key={item.kind} value={item.kind}>{item.label}</option>
          ))}
        </select>
        <button type="submit" className="h-6 rounded px-1.5 text-koma-dim hover:bg-koma-hover hover:text-koma-fg">Add</button>
      </form>
      {error ? <p className="text-[11px] text-koma-error">{error}</p> : null}
      {doc.tokens.length === 0 ? <p className="text-koma-dim">No tokens</p> : null}
      {TOKEN_KINDS.map((group) => {
        const tokens = doc.tokens.filter((token) => token.kind === group.kind)
        if (!tokens.length) return null
        return (
          <div key={group.kind} className="flex flex-col">
            <span className="px-0.5 pt-1 text-[11px] text-koma-dim">{group.label}</span>
            {tokens.map((token) => (
              <div key={`${root}:${path}:${token.name}`} className="flex h-7 min-w-0 items-center gap-1">
                <span className="min-w-0 flex-1 truncate" title={token.name}>{token.name}</span>
                <TokenValue
                  token={token}
                  mode={doc.mode}
                  onValue={(value) => {
                    const next = setDesignTokenValue(doc, token.name, doc.mode, value)
                    if (next) onCommit(next)
                  }}
                />
                <IconBtn label={`Delete ${token.name}`} tone="red" onClick={() => onCommit(dropDesignToken(doc, token.name))}>
                  <Trash2 size={12} />
                </IconBtn>
              </div>
            ))}
          </div>
        )
      })}
    </div>
  )
}

function rootLabel(root: string): string {
  const parts = root.split('/').filter(Boolean)
  return parts[parts.length - 1] || root
}

function missingDir(error: string | null | undefined): boolean {
  if (!error) return false
  return /no such file|not found|os error 2/i.test(error)
}

function InlineNameInput({
  initial,
  placeholder,
  onSubmit,
  onCancel,
}: {
  initial: string
  placeholder: string
  onSubmit: (value: string) => void
  onCancel: () => void
}) {
  const [value, setValue] = useState(initial)
  useEffect(() => {
    const el = document.getElementById('design-name-input') as HTMLInputElement | null
    el?.focus()
    el?.select()
  }, [])
  const commit = () => {
    const next = value.trim()
    if (!next) {
      onCancel()
      return
    }
    onSubmit(next)
  }
  return (
    <form
      className="flex min-w-0 flex-1 items-center gap-1"
      onSubmit={(e: FormEvent) => {
        e.preventDefault()
        commit()
      }}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
    >
      <input
        id="design-name-input"
        value={value}
        placeholder={placeholder}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e: KeyboardEvent<HTMLInputElement>) => {
          if (e.key === 'Escape') {
            e.preventDefault()
            onCancel()
          }
        }}
        onBlur={() => {
          if (!value.trim()) onCancel()
        }}
        className="h-5 min-w-0 flex-1 rounded border border-koma-border bg-koma-bg px-1.5 text-[12px] text-koma-fg outline-none focus:border-koma-fg/40"
      />
      <IconBtn label="Confirm" tone="emerald" onClick={commit}>
        <Check size={12} />
      </IconBtn>
      <IconBtn label="Cancel" tone="red" onClick={onCancel}>
        <X size={12} />
      </IconBtn>
    </form>
  )
}

export function DesignPanel() {
  const sessionId = useKoma((s) => s.session.id)
  const workdir = useKoma((s) => s.settingsValues?.workdir ?? EMPTY_ROOTS)
  const activeRoot = useKoma((s) => s.coding.activeRoot)
  const dir = useKoma((s) => (activeRoot ? s.coding.dirs[fileKey(activeRoot, '.koma')] : undefined))
  const docs = useKoma((s) => s.design.docs)
  const setActiveCodingRoot = useKoma((s) => s.setActiveCodingRoot)
  const refreshCodingDir = useKoma((s) => s.refreshCodingDir)
  const openDesignTab = useKoma((s) => s.openDesignTab)
  const updateDesign = useKoma((s) => s.updateDesign)
  const createDesignFile = useKoma((s) => s.createDesignFile)
  const designTab = useKoma((s) => {
    const tab = s.ui.tabs.find((item) => item.id === s.ui.activeTabId)
    return tab && tab.kind === 'design' ? tab : null
  })
  const renameCodingItem = useKoma((s) => s.renameCodingItem)
  const deleteCodingItem = useKoma((s) => s.deleteCodingItem)
  const req = useKoma((s) => s.req)

  const [shapesOpen, setShapesOpen] = useState(true)
  const [tokensOpen, setTokensOpen] = useState(true)
  const [componentsOpen, setComponentsOpen] = useState(true)
  const [filesOpen, setFilesOpen] = useState(true)
  const [creating, setCreating] = useState(false)
  const [renaming, setRenaming] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<string | null>(null)
  const [menu, setMenu] = useState<null | { x: number; y: number; path: string }>(null)

  useEffect(() => {
    req({ r: 'GetSettings' })
  }, [req])

  useEffect(() => {
    if (workdir.length === 0) {
      if (activeRoot != null) setActiveCodingRoot(null)
      return
    }
    if (!activeRoot || !workdir.includes(activeRoot)) setActiveCodingRoot(workdir[0])
  }, [workdir, activeRoot, setActiveCodingRoot])

  useEffect(() => {
    if (!activeRoot) return
    refreshCodingDir(activeRoot, '.koma')
    setCreating(false)
    setRenaming(null)
    setDeleting(null)
    setMenu(null)
  }, [activeRoot, refreshCodingDir])

  useEffect(() => {
    if (!menu) return
    const close = () => setMenu(null)
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    window.addEventListener('click', close)
    window.addEventListener('keydown', onKey)
    window.addEventListener('blur', close)
    return () => {
      window.removeEventListener('click', close)
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('blur', close)
    }
  }, [menu])

  if (sessionId === null) return <Empty>Open a project to use Design</Empty>
  if (workdir.length === 0) return <Empty>No workspaces configured. Add paths under Settings → Session → workdir.</Empty>

  const files: FileTreeEntry[] = (dir?.entries ?? []).filter((entry) => !entry.isDir && isDesignPath(entry.path))
  const gone = missingDir(dir?.error)
  const busy = (path: string) => {
    if (!activeRoot) return false
    return !!docs[fileKey(activeRoot, path)]?.saving
  }

  const submitCreate = (raw: string) => {
    if (!activeRoot) return
    const name = designFileName(raw)
    if (!name) return
    createDesignFile(activeRoot, `.koma/${name}`)
    setCreating(false)
  }

  const submitRename = (path: string, raw: string) => {
    if (!activeRoot || busy(path)) return
    const name = designFileName(raw)
    if (!name) return
    const next = `.koma/${name}`
    if (next !== path) renameCodingItem(activeRoot, path, next)
    setRenaming(null)
  }

  const confirmDelete = (path: string) => {
    if (!activeRoot || busy(path)) return
    deleteCodingItem(activeRoot, path)
    setDeleting(null)
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-none items-center gap-1 px-2 py-1.5">
        <div className="min-w-0 flex-1" title={activeRoot ?? ''}>
          <Select
            value={activeRoot ?? workdir[0] ?? ''}
            options={workdir.map((root) => ({ value: root, label: rootLabel(root) }))}
            onChange={(root) => setActiveCodingRoot(root)}
            disabled={creating || renaming != null || deleting != null}
          />
        </div>
      </div>
      <AccordionSection title="Shapes" open={shapesOpen} onToggle={() => setShapesOpen((open) => !open)} fill={false}>
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5 px-2 py-1.5">
          {SHAPES.map((shape) => (
            <ShapeTile key={shape.kind} {...shape} />
          ))}
        </div>
      </AccordionSection>
      <AccordionSection title="Tokens" open={tokensOpen} onToggle={() => setTokensOpen((open) => !open)} fill={false}>
        {!designTab ? (
          <p className="px-3 py-1.5 text-[12px] text-koma-dim">Open a design to edit its tokens</p>
        ) : !docs[fileKey(designTab.root, designTab.path)] || docs[fileKey(designTab.root, designTab.path)]?.loading ? (
          <div className="flex items-center gap-2 px-3 py-1.5 text-[12px] text-koma-dim">
            <BrailleSpinner size={13} />
            <span>Loading…</span>
          </div>
        ) : (
          <TokenEditor
            root={designTab.root}
            path={designTab.path}
            doc={docs[fileKey(designTab.root, designTab.path)]!.doc}
            onCommit={(next) => commitDesign(designTab.root, designTab.path, next, updateDesign)}
          />
        )}
      </AccordionSection>
      <AccordionSection title="Components" open={componentsOpen} onToggle={() => setComponentsOpen((open) => !open)} fill={false}>
        {!designTab ? (
          <p className="px-3 py-1.5 text-[12px] text-koma-dim">Open a design to use its components</p>
        ) : !docs[fileKey(designTab.root, designTab.path)] || docs[fileKey(designTab.root, designTab.path)]?.loading ? (
          <div className="flex items-center gap-2 px-3 py-1.5 text-[12px] text-koma-dim">
            <BrailleSpinner size={13} />
            <span>Loading…</span>
          </div>
        ) : docs[fileKey(designTab.root, designTab.path)]!.doc.components.length === 0 ? (
          <p className="px-3 py-1.5 text-[12px] text-koma-dim">No components</p>
        ) : (
          docs[fileKey(designTab.root, designTab.path)]!.doc.components.map((component) => (
            <div
              key={component.id}
              draggable
              title="Drag onto a screen"
              onDragStart={(event) => {
                event.dataTransfer.setData(COMPONENT_MIME, component.id)
                event.dataTransfer.setData('text/plain', component.id)
                try {
                  event.dataTransfer.effectAllowed = 'copy'
                } catch {
                  /* ignore */
                }
              }}
              className="flex h-7 cursor-grab items-center px-2 text-[12px] text-koma-fg hover:bg-koma-hover active:cursor-grabbing"
            >
              <button
                type="button"
                className="min-w-0 flex-1 truncate text-left"
                onClick={() => {
                  if (!designTab) return
                  openDesignTab(designTab.root, designTab.path)
                  const detail = { root: designTab.root, path: designTab.path, componentId: component.id }
                  window.setTimeout(() => {
                    window.dispatchEvent(new CustomEvent('koma-design-focus', { detail }))
                  }, 0)
                }}
              >
                {component.name}
              </button>
              <button
                type="button"
                title="Add to chat"
                className="flex-none rounded px-1 text-[11px] text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
                onClick={(event) => {
                  event.stopPropagation()
                  const open = docs[fileKey(designTab.root, designTab.path)]
                  if (!open) return
                  const text = designChatText(open.doc, { component: component.id })
                  if (!text) return
                  const state = useKoma.getState()
                  state.appendToComposer(text)
                  state.activateTab('chat')
                }}
              >
                Chat
              </button>
            </div>
          ))
        )}
      </AccordionSection>
      <AccordionSection
        title="Designs"
        open={filesOpen}
        onToggle={() => setFilesOpen((open) => !open)}
        action={<AddBtn label="New design" onClick={() => { setCreating(true); setRenaming(null); setDeleting(null) }} />}
      >
        {!activeRoot ? (
          <Empty>Select a workspace root</Empty>
        ) : dir?.loading && !dir.entries.length && !creating ? (
          <div className="flex items-center gap-2 px-3 py-2 text-[12px] text-koma-dim">
            <BrailleSpinner size={13} />
            <span>Loading…</span>
          </div>
        ) : dir?.error && !gone ? (
          <div className="px-3 py-2 text-[12px] text-koma-error">{dir.error}</div>
        ) : (
          <>
            {creating ? (
              <div className="flex h-7 min-w-0 items-center gap-1 px-2 text-[12px] text-koma-fg">
                <File size={13} className="flex-none opacity-70" />
                <InlineNameInput initial="" placeholder="name.kdsgn" onSubmit={submitCreate} onCancel={() => setCreating(false)} />
              </div>
            ) : null}
            {files.length ? (
              files.map((entry) => {
                const dirty = activeRoot ? !!docs[fileKey(activeRoot, entry.path)]?.dirty : false
                if (deleting === entry.path) {
                  return (
                    <div
                      key={entry.path}
                      className="flex min-h-[28px] w-full items-center gap-2 bg-koma-error/15 px-2 text-[12px] font-medium text-koma-error"
                    >
                      <span className="min-w-0 flex-1 truncate">delete file?</span>
                      <button type="button" className="rounded px-1.5 py-0.5 text-koma-success hover:bg-koma-success/15" onClick={() => confirmDelete(entry.path)}>
                        yes
                      </button>
                      <button type="button" className="rounded px-1.5 py-0.5 text-koma-error hover:bg-koma-error/15" onClick={() => setDeleting(null)}>
                        no
                      </button>
                    </div>
                  )
                }
                return (
                  <div
                    key={entry.path}
                    className="group flex h-7 min-w-0 items-center gap-1 px-2 text-[12px] text-koma-fg hover:bg-koma-hover"
                    onContextMenu={(e: ReactMouseEvent) => {
                      e.preventDefault()
                      setMenu({ x: e.clientX, y: e.clientY, path: entry.path })
                    }}
                  >
                    {renaming === entry.path ? (
                      <>
                        <File size={13} className="flex-none opacity-70" />
                        <InlineNameInput
                          initial={entry.name}
                          placeholder="new name"
                          onSubmit={(name) => submitRename(entry.path, name)}
                          onCancel={() => setRenaming(null)}
                        />
                      </>
                    ) : (
                      <>
                        <button
                          type="button"
                          className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden text-left"
                          title={entry.path}
                          onClick={() => activeRoot && openDesignTab(activeRoot, entry.path)}
                        >
                          <File size={13} className="flex-none opacity-70" />
                          <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                        </button>
                        <div className="flex max-w-0 flex-none items-center overflow-hidden opacity-0 transition-[max-width,opacity] duration-100 group-hover:max-w-[56px] group-hover:opacity-100">
                          <IconBtn label="Rename" onClick={() => { setRenaming(entry.path); setCreating(false); setDeleting(null) }}>
                            <Pencil size={12} />
                          </IconBtn>
                          <IconBtn label="Delete" tone="red" onClick={() => { setDeleting(entry.path); setCreating(false); setRenaming(null) }}>
                            <Trash2 size={12} />
                          </IconBtn>
                        </div>
                        {dirty ? <span className="flex-none font-mono text-[11px] font-semibold text-koma-accent">M</span> : null}
                      </>
                    )}
                  </div>
                )
              })
            ) : !creating ? (
              <Empty>No designs</Empty>
            ) : null}
          </>
        )}
      </AccordionSection>
      {menu ? (
        <div
          className="fixed z-[80] min-w-[140px] rounded border border-koma-border bg-koma-panel py-1 shadow-lg"
          style={{ left: menu.x, top: menu.y }}
          onClick={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          <button
            type="button"
            className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-fg opacity-90 hover:bg-koma-hover"
            onClick={() => {
              setRenaming(menu.path)
              setCreating(false)
              setDeleting(null)
              setMenu(null)
            }}
          >
            <Pencil size={12} className="opacity-70" />
            Rename
          </button>
          <button
            type="button"
            className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[12px] text-koma-error opacity-90 hover:bg-koma-error/15"
            onClick={() => {
              setDeleting(menu.path)
              setCreating(false)
              setRenaming(null)
              setMenu(null)
            }}
          >
            <Trash2 size={12} className="opacity-70" />
            Delete
          </button>
        </div>
      ) : null}
    </div>
  )
}
