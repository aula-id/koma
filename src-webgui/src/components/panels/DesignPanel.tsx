import { useEffect, useRef, useState, useSyncExternalStore, type DragEvent as ReactDragEvent, type FormEvent, type KeyboardEvent, type MouseEvent as ReactMouseEvent } from 'react'
import { Check, Component, File, MessageSquare, Pencil, Plus, Trash2, X } from 'lucide-react'
import { AccordionSection } from '../AccordionSection'
import { BrailleSpinner } from '../BrailleSpinner'
import { DesignLayers } from '../DesignLayers'
import { Empty, IconBtn } from './helpers'
import { Select } from './form'
import { useKoma } from '../../store/koma'
import { fileKey, type FileTreeEntry } from '../../store/coding'
import {
  COMPONENT_MIME,
  addDesignToken,
  componentView,
  designChatText,
  designFileName,
  dropDesignToken,
  isDesignPath,
  resolveRef,
  setDesignMode,
  setDesignTokenValue,
  type DesignDoc,
  type DesignNode,
  type DesignToken,
  type DesignTokenKind,
} from '../../lib/design'
import { emitDesignLayer, getDesignUi, subscribeDesignUi } from '../../lib/designUi'

const EMPTY_ROOTS: string[] = []
const TOKEN_KINDS: { kind: DesignTokenKind; label: string }[] = [
  { kind: 'color', label: 'Color' },
  { kind: 'space', label: 'Space' },
  { kind: 'type', label: 'Type' },
  { kind: 'radius', label: 'Radius' },
]

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

export function TokenEditor({ root, path, doc, query = '', onCommit }: { root: string; path: string; doc: DesignDoc; query?: string; onCommit: (doc: DesignDoc) => void }) {
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
        const tokens = doc.tokens.filter((token) => token.kind === group.kind && (!query || token.name.toLowerCase().includes(query)))
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
  const createDesignFile = useKoma((s) => s.createDesignFile)
  const designTab = useKoma((s) => {
    const tab = s.ui.tabs.find((item) => item.id === s.ui.activeTabId)
    return tab && tab.kind === 'design' ? tab : null
  })
  const renameCodingItem = useKoma((s) => s.renameCodingItem)
  const deleteCodingItem = useKoma((s) => s.deleteCodingItem)
  const req = useKoma((s) => s.req)

  const [panel, setPanel] = useState<'file' | 'assets'>('file')
  const [filesOpen, setFilesOpen] = useState(true)
  const [layersOpen, setLayersOpen] = useState(true)
  const [assetQuery, setAssetQuery] = useState('')
  const [creating, setCreating] = useState(false)
  const ui = useSyncExternalStore(subscribeDesignUi, getDesignUi, getDesignUi)
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

  const open = designTab ? docs[fileKey(designTab.root, designTab.path)] : undefined
  const sameFile = !!(ui && designTab && ui.root === designTab.root && ui.path === designTab.path)
  const focusId = sameFile && ui ? ui.focusId : null
  const selection = sameFile && ui ? ui.selection : []
  const viewDoc = open && !open.loading ? (focusId ? componentView(open.doc, focusId) ?? open.doc : open.doc) : null
  const query = assetQuery.trim().toLowerCase()
  const components = (open?.doc.components ?? []).filter((component) => !query || component.name.toLowerCase().includes(query))

  return (
    <div className="flex h-full min-h-0 flex-col" onContextMenu={(event) => event.preventDefault()}>
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
      <div className="mx-2 mb-1 flex h-7 flex-none rounded-md bg-koma-bg p-0.5">
        <button
          type="button"
          aria-pressed={panel === 'file'}
          onClick={() => setPanel('file')}
          className={`flex-1 rounded text-[12px] ${panel === 'file' ? 'bg-koma-panel text-koma-fg shadow-sm' : 'text-koma-dim hover:text-koma-fg'}`}
        >
          File
        </button>
        <button
          type="button"
          aria-pressed={panel === 'assets'}
          onClick={() => setPanel('assets')}
          className={`flex-1 rounded text-[12px] ${panel === 'assets' ? 'bg-koma-panel text-koma-fg shadow-sm' : 'text-koma-dim hover:text-koma-fg'}`}
        >
          Assets
        </button>
      </div>
      {panel === 'file' ? (
        <div className="flex min-h-0 flex-1 flex-col">
          <AccordionSection
            title="Files"
            open={filesOpen}
            onToggle={() => setFilesOpen((value) => !value)}
            fill={false}
            action={(
              <button
                type="button"
                aria-label="New design"
                title="New design"
                onClick={() => { setCreating(true); setRenaming(null); setDeleting(null) }}
                className="flex h-5 w-5 items-center justify-center rounded text-koma-dim hover:bg-koma-hover hover:text-koma-fg"
              >
                <Plus size={13} />
              </button>
            )}
          >
            <div className="max-h-40 overflow-y-auto">
              <FileList
                activeRoot={activeRoot}
                files={files}
                creating={creating}
                renaming={renaming}
                deleting={deleting}
                currentPath={designTab && designTab.root === activeRoot ? designTab.path : null}
                loading={!!dir?.loading && !dir.entries.length && !creating}
                error={dir?.error && !gone ? dir.error : null}
                dirty={(path) => !!activeRoot && !!docs[fileKey(activeRoot, path)]?.dirty}
                onOpen={(path) => activeRoot && openDesignTab(activeRoot, path)}
                onCreate={submitCreate}
                onCancelCreate={() => setCreating(false)}
                onRename={submitRename}
                onCancelRename={() => setRenaming(null)}
                onAskRename={(path) => { setRenaming(path); setCreating(false); setDeleting(null) }}
                onAskDelete={(path) => { setDeleting(path); setCreating(false); setRenaming(null) }}
                onCancelDelete={() => setDeleting(null)}
                onConfirmDelete={confirmDelete}
                onMenu={(path, x, y) => setMenu({ x, y, path })}
              />
            </div>
          </AccordionSection>
          <AccordionSection title="Layers" open={layersOpen} onToggle={() => setLayersOpen((value) => !value)}>
            {open?.loading ? (
              <div className="flex items-center gap-2 px-3 py-1.5 text-[12px] text-koma-dim">
                <BrailleSpinner size={13} />
                <span>Loading…</span>
              </div>
            ) : viewDoc && designTab ? (
              <DesignLayers
                doc={viewDoc}
                selection={selection}
                onSelect={(id, shift) => emitDesignLayer(designTab.root, designTab.path, { op: 'select', id, shift })}
                onRename={(id, name) => emitDesignLayer(designTab.root, designTab.path, { op: 'rename', id, name })}
                onVisible={(id, visible) => emitDesignLayer(designTab.root, designTab.path, { op: 'visible', id, visible })}
                onLocked={(id, locked) => emitDesignLayer(designTab.root, designTab.path, { op: 'locked', id, locked })}
                onMove={(id, parentId, index) => emitDesignLayer(designTab.root, designTab.path, { op: 'move', id, parentId, index })}
                onMenu={(id, x, y) => emitDesignLayer(designTab.root, designTab.path, { op: 'menu', id, x, y })}
              />
            ) : (
              <p className="px-3 text-[12px] text-koma-dim">Open a design to see its layers</p>
            )}
          </AccordionSection>
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col">
          <input
            value={assetQuery}
            onChange={(event) => setAssetQuery(event.target.value)}
            placeholder="Search"
            aria-label="Search assets"
            className="mx-2 mt-2 h-7 flex-none rounded border border-koma-border bg-koma-bg px-2 text-[12px] text-koma-fg outline-none"
          />
          {!designTab || !open || open.loading ? (
            <p className="px-3 py-2 text-[12px] text-koma-dim">{designTab ? 'Loading…' : 'Open a design to use its assets'}</p>
          ) : (
            <div className="min-h-0 flex-1 overflow-y-auto pb-3">
              <div className="px-3 pb-1 pt-3 text-[11px] text-koma-dim">Components</div>
              {components.length === 0 ? <p className="px-3 text-[12px] text-koma-dim">No components</p> : (
                <div className="grid grid-cols-2 gap-1.5 px-2">
                  {components.map((component) => (
                    <AssetTile
                      key={component.id}
                      name={component.name}
                      doc={open.doc}
                      node={component.variants[0]?.node}
                      onOpen={() => {
                        openDesignTab(designTab.root, designTab.path)
                        const detail = { root: designTab.root, path: designTab.path, componentId: component.id }
                        window.setTimeout(() => {
                          window.dispatchEvent(new CustomEvent('koma-design-focus', { detail }))
                        }, 0)
                      }}
                      onDragStart={(event) => {
                        event.dataTransfer.setData(COMPONENT_MIME, component.id)
                        event.dataTransfer.setData('text/plain', component.id)
                        try {
                          event.dataTransfer.effectAllowed = 'copy'
                        } catch {
                          /* ignore */
                        }
                      }}
                      onChat={() => {
                        const text = designChatText(open.doc, { component: component.id })
                        if (!text) return
                        useKoma.getState().addDesignToChat({ title: component.name, text })
                      }}
                    />
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}
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

function FileList({
  activeRoot,
  files,
  creating,
  renaming,
  deleting,
  currentPath,
  loading,
  error,
  dirty,
  onOpen,
  onCreate,
  onCancelCreate,
  onRename,
  onCancelRename,
  onAskRename,
  onAskDelete,
  onCancelDelete,
  onConfirmDelete,
  onMenu,
}: {
  activeRoot: string | null
  files: FileTreeEntry[]
  creating: boolean
  renaming: string | null
  deleting: string | null
  currentPath: string | null
  loading: boolean
  error: string | null
  dirty: (path: string) => boolean
  onOpen: (path: string) => void
  onCreate: (name: string) => void
  onCancelCreate: () => void
  onRename: (path: string, name: string) => void
  onCancelRename: () => void
  onAskRename: (path: string) => void
  onAskDelete: (path: string) => void
  onCancelDelete: () => void
  onConfirmDelete: (path: string) => void
  onMenu: (path: string, x: number, y: number) => void
}) {
  if (!activeRoot) return <Empty>Select a workspace root</Empty>
  if (loading) {
    return (
      <div className="flex items-center gap-2 px-3 py-2 text-[12px] text-koma-dim">
        <BrailleSpinner size={13} />
        <span>Loading…</span>
      </div>
    )
  }
  if (error) return <div className="px-3 py-2 text-[12px] text-koma-error">{error}</div>
  return (
    <>
      {creating ? (
        <div className="flex h-7 min-w-0 items-center gap-1 px-2 text-[12px] text-koma-fg">
          <File size={13} className="flex-none opacity-70" />
          <InlineNameInput initial="" placeholder="name.kdsgn" onSubmit={onCreate} onCancel={onCancelCreate} />
        </div>
      ) : null}
      {files.length ? files.map((entry) => {
        if (deleting === entry.path) {
          return (
            <div key={entry.path} className="flex min-h-[28px] w-full items-center gap-2 bg-koma-error/15 px-2 text-[12px] font-medium text-koma-error">
              <span className="min-w-0 flex-1 truncate">delete file?</span>
              <button type="button" className="rounded px-1.5 py-0.5 text-koma-success hover:bg-koma-success/15" onClick={() => onConfirmDelete(entry.path)}>yes</button>
              <button type="button" className="rounded px-1.5 py-0.5 text-koma-error hover:bg-koma-error/15" onClick={onCancelDelete}>no</button>
            </div>
          )
        }
        const current = entry.path === currentPath
        return (
          <div
            key={entry.path}
            className={`group flex h-7 min-w-0 items-center gap-1 px-2 text-[12px] text-koma-fg ${current ? 'bg-koma-accent/15' : 'hover:bg-koma-hover'}`}
            onContextMenu={(event: ReactMouseEvent) => {
              event.preventDefault()
              onMenu(entry.path, event.clientX, event.clientY)
            }}
          >
            {renaming === entry.path ? (
              <>
                <File size={13} className="flex-none opacity-70" />
                <InlineNameInput initial={entry.name} placeholder="new name" onSubmit={(name) => onRename(entry.path, name)} onCancel={onCancelRename} />
              </>
            ) : (
              <>
                <button type="button" className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden text-left" title={entry.path} onClick={() => onOpen(entry.path)}>
                  <File size={13} className="flex-none opacity-70" />
                  <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                </button>
                <div className="flex max-w-0 flex-none items-center overflow-hidden opacity-0 transition-[max-width,opacity] duration-100 group-hover:max-w-[56px] group-hover:opacity-100">
                  <IconBtn label="Rename" onClick={() => onAskRename(entry.path)}><Pencil size={12} /></IconBtn>
                  <IconBtn label="Delete" tone="red" onClick={() => onAskDelete(entry.path)}><Trash2 size={12} /></IconBtn>
                </div>
                {dirty(entry.path) ? <span className="h-1.5 w-1.5 flex-none rounded-full bg-koma-accent" title="Unsaved" /> : null}
              </>
            )}
          </div>
        )
      }) : !creating ? <Empty>No designs</Empty> : null}
    </>
  )
}

function AssetTile({
  name,
  doc,
  node,
  onOpen,
  onDragStart,
  onChat,
}: {
  name: string
  doc: DesignDoc
  node?: DesignNode
  onOpen: () => void
  onDragStart: (event: ReactDragEvent<HTMLButtonElement>) => void
  onChat: () => void
}) {
  const dragged = useRef(false)
  return (
    <div className="group relative">
      <button
        type="button"
        draggable
        title={name}
        onClick={() => {
          if (dragged.current) return
          onOpen()
        }}
        onDragStart={(event) => {
          dragged.current = true
          onDragStart(event)
        }}
        onDragEnd={() => {
          setTimeout(() => {
            dragged.current = false
          }, 0)
        }}
        className="flex w-full cursor-grab flex-col items-stretch gap-1 rounded p-1 text-left hover:bg-koma-hover active:cursor-grabbing"
      >
        {node ? <AssetThumb doc={doc} node={node} /> : (
          <span className="flex h-[72px] w-full items-center justify-center rounded bg-[#e6e8ed] text-[#9747ff]">
            <Component size={16} />
          </span>
        )}
        <span className="flex items-center gap-1 px-0.5 text-[11px] text-koma-fg">
          <Component size={11} className="flex-none text-[#9747ff]" />
          <span className="min-w-0 flex-1 truncate">{name}</span>
        </span>
      </button>
      <button
        type="button"
        title="Add to chat"
        aria-label={`Add ${name} to chat`}
        onClick={onChat}
        className="absolute right-1.5 top-1.5 hidden h-5 w-5 items-center justify-center rounded bg-white text-koma-dim shadow-sm hover:text-koma-fg group-hover:flex"
      >
        <MessageSquare size={12} />
      </button>
    </div>
  )
}

function AssetThumb({ doc, node }: { doc: DesignDoc; node: DesignNode }) {
  const scale = Math.min(64 / Math.max(node.w, 1), 48 / Math.max(node.h, 1))
  return (
    <div className="flex h-[72px] w-full items-center justify-center overflow-hidden rounded bg-[#e6e8ed]">
      <div style={{ width: node.w * scale, height: node.h * scale }}>
        <div style={{ width: node.w, height: node.h, transform: `scale(${scale})`, transformOrigin: 'top left' }}>
          <ThumbNode doc={doc} node={{ ...node, x: 0, y: 0 }} />
        </div>
      </div>
    </div>
  )
}

function ThumbNode({ doc, node }: { doc: DesignDoc; node: DesignNode }) {
  if (node.visible === false) return null
  const painted = !node.fill || node.fill === 'none' ? '' : node.fill.startsWith('#') ? node.fill : resolveRef(doc, node.fill)
  const fill = node.fill === 'none' ? 'transparent' : painted || (node.kind === 'frame' ? '#ffffff' : node.kind === 'rect' || node.kind === 'ellipse' ? '#d9d9d9' : 'transparent')
  const radius = node.kind === 'ellipse' ? '50%' : typeof node.radius === 'number' ? node.radius : 0
  return (
    <div
      className="absolute overflow-hidden"
      style={{
        left: node.x,
        top: node.y,
        width: node.w,
        height: node.h,
        background: node.kind === 'line' || node.kind === 'vector' || node.kind === 'text' ? 'transparent' : fill,
        borderRadius: radius,
      }}
    >
      {node.kind === 'text' ? <span className="block truncate px-1 text-[13px] text-[#1c1c1c]">{node.text || 'Text'}</span> : null}
      {node.children?.map((child) => <ThumbNode key={child.id} doc={doc} node={child} />)}
    </div>
  )
}
