import { useEffect, useRef, useState } from 'react'
import { File, Frame, Shapes } from 'lucide-react'
import { InlineNameInput } from './panels/helpers'
import { Select } from './panels/form'
import { useKoma } from '../store/koma'
import { fileKey } from '../store/coding'
import { pathBasename } from '../lib/pathDisplay'
import { designFileName, isDesignPath } from '../lib/design'
import { diagramFileName, isDiagramPath } from '../lib/diagram'
import { importFigFile } from '../lib/design/importFigFile'
import { recentWelcomeFiles, type WelcomeWhich } from '../lib/welcome'

const EMPTY_ROOTS: string[] = []

const COPY = {
  diagram: {
    Icon: Shapes,
    heading: 'Diagrams',
    noun: 'diagram',
    ext: '.diag',
    text: 'Draw flowcharts, ER diagrams and architecture maps on a canvas. Each diagram is a .diag file in the .koma folder of your workspace, and your agent can read and edit it.',
    steps: [
      'Drag a shape from the Diagram sidebar onto the canvas',
      'Drag from a shape edge to connect two shapes',
      'Paste CREATE TABLE SQL or JSON onto the canvas to import it',
      'Ask the agent: draw the auth flow',
    ],
  },
  design: {
    Icon: Frame,
    heading: 'Design',
    noun: 'design',
    ext: '.kdsgn',
    text: 'Lay out screens and components with frames, tokens and reusable components. Each design is a .kdsgn file in the .koma folder of your workspace, and your agent can read and edit it.',
    steps: ['Drag a frame onto the canvas', 'Define tokens and components in the sidebar', 'Ask the agent to design a screen'],
  },
} as const

const rootLabel = pathBasename
const baseName = (path: string): string => path.slice(path.lastIndexOf('/') + 1)

const BTN = 'rounded border border-koma-border px-3 py-1 text-[12px] text-koma-fg hover:bg-koma-hover disabled:cursor-not-allowed disabled:opacity-50'

export default function WelcomeTab({ which }: { which: WelcomeWhich }) {
  const copy = COPY[which]
  const sessionId = useKoma((s) => s.session.id)
  const workdir = useKoma((s) => s.settingsValues?.workdir ?? EMPTY_ROOTS)
  const activeRoot = useKoma((s) => s.coding.activeRoot)
  const dir = useKoma((s) => (activeRoot ? s.coding.dirs[fileKey(activeRoot, '.koma')] : undefined))
  const pending = useKoma((s) => (which === 'diagram' ? s.diagram.pendingCreate : s.design?.pendingCreate) ?? null)
  const setActiveCodingRoot = useKoma((s) => s.setActiveCodingRoot)
  const refreshCodingDir = useKoma((s) => s.refreshCodingDir)
  const createDiagramFile = useKoma((s) => s.createDiagramFile)
  const createDesignFile = useKoma((s) => s.createDesignFile)
  const openDiagramTab = useKoma((s) => s.openDiagramTab)
  const openDesignTab = useKoma((s) => s.openDesignTab)
  const openSettingsTab = useKoma((s) => s.openSettingsTab)
  const req = useKoma((s) => s.req)
  const [creating, setCreating] = useState(false)
  const [listOpen, setListOpen] = useState(false)
  const importRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    req({ r: 'GetSettings' })
  }, [req])

  useEffect(() => {
    if (workdir.length === 0) return
    if (!activeRoot || !workdir.includes(activeRoot)) setActiveCodingRoot(workdir[0])
  }, [workdir, activeRoot, setActiveCodingRoot])

  useEffect(() => {
    if (!activeRoot) return
    refreshCodingDir(activeRoot, '.koma')
    setCreating(false)
  }, [activeRoot, refreshCodingDir])

  const label = which === 'diagram' ? 'Diagram' : 'Design'
  if (sessionId === null) return <Centered>Open a project to use {label}</Centered>
  if (workdir.length === 0) {
    return (
      <Centered>
        <p>No workspaces configured. Add paths under Settings → Session → workdir.</p>
        <button type="button" onClick={openSettingsTab} className={`${BTN} mt-3`}>
          Open Settings
        </button>
      </Centered>
    )
  }

  const match = which === 'diagram' ? isDiagramPath : isDesignPath
  const files = (dir?.entries ?? []).filter((e) => !e.isDir && match(e.path)).map((e) => e.path)
  const shown = listOpen ? files : recentWelcomeFiles(files)
  const openFile = (path: string) => {
    if (!activeRoot) return
    if (which === 'diagram') openDiagramTab(activeRoot, path)
    else openDesignTab(activeRoot, path)
  }
  const submitCreate = (raw: string) => {
    if (!activeRoot) return
    const name = which === 'diagram' ? diagramFileName(raw) : designFileName(raw)
    if (!name) return
    const path = `.koma/${name}`
    if (which === 'diagram') createDiagramFile(activeRoot, path)
    else createDesignFile(activeRoot, path)
    setCreating(false)
  }

  return (
    <div className="h-full min-w-0 overflow-y-auto bg-koma-bg text-koma-fg">
      <div className="mx-auto flex max-w-xl flex-col gap-5 px-6 py-8">
        {workdir.length > 1 ? (
          <div title={activeRoot ?? ''}>
            <Select
              value={activeRoot ?? workdir[0] ?? ''}
              options={workdir.map((r) => ({ value: r, label: rootLabel(r) }))}
              onChange={(root) => setActiveCodingRoot(root)}
              disabled={creating}
            />
          </div>
        ) : null}
        <div className="flex items-center gap-2">
          <copy.Icon size={22} className="flex-none opacity-80" />
          <h1 className="text-[20px] font-semibold">{copy.heading}</h1>
        </div>
        <p className="text-[13px] leading-relaxed text-koma-dim">{copy.text}</p>
        <div className="flex flex-wrap items-center gap-2">
          <button type="button" className={BTN} disabled={pending !== null} onClick={() => setCreating(true)}>
            New {copy.noun}
          </button>
          <button type="button" className={BTN} aria-pressed={listOpen} onClick={() => setListOpen((v) => !v)}>
            Open existing
          </button>
          {which === 'design' ? (
            <>
              <button type="button" className={BTN} onClick={() => importRef.current?.click()}>
                Import .fig
              </button>
              <input
                ref={importRef}
                type="file"
                accept=".fig,application/octet-stream"
                hidden
                onChange={(event) => {
                  const file = event.target.files?.[0]
                  event.target.value = ''
                  if (!file || !activeRoot) return
                  void importFigFile(file, activeRoot, req, openDesignTab)
                }}
              />
            </>
          ) : null}
        </div>
        {creating ? (
          <div className="flex h-7 min-w-0 items-center gap-1 text-[12px]">
            <File size={13} className="flex-none opacity-70" />
            <InlineNameInput initial="" placeholder={`name${copy.ext}`} onSubmit={submitCreate} onCancel={() => setCreating(false)} />
          </div>
        ) : null}
        {listOpen || files.length > 0 ? (
          <section className="flex flex-col gap-1">
            <h2 className="text-[11px] font-semibold uppercase tracking-wider text-koma-dim">{listOpen ? 'Existing files' : 'Recent files'}</h2>
            {shown.length ? (
              shown.map((path) => (
                <button
                  key={path}
                  type="button"
                  title={path}
                  onClick={() => openFile(path)}
                  className="flex h-7 min-w-0 items-center gap-2 rounded px-2 text-left text-[12px] hover:bg-koma-hover"
                >
                  <File size={13} className="flex-none opacity-70" />
                  <span className="truncate">{baseName(path)}</span>
                </button>
              ))
            ) : (
              <p className="text-[12px] text-koma-dim">No {which === 'diagram' ? 'diagrams' : 'designs'} yet</p>
            )}
          </section>
        ) : null}
        <section className="flex flex-col gap-1">
          <h2 className="text-[11px] font-semibold uppercase tracking-wider text-koma-dim">How it works</h2>
          <ul className="list-disc pl-5 text-[13px] leading-relaxed">
            {copy.steps.map((step) => (
              <li key={step}>{step}</li>
            ))}
          </ul>
        </section>
      </div>
    </div>
  )
}

function Centered({ children }: { children: React.ReactNode }) {
  return <div className="flex h-full flex-col items-center justify-center bg-koma-bg px-6 text-center text-[13px] text-koma-dim">{children}</div>
}
