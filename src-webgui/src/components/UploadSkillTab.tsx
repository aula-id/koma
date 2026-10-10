import { notificationOrigin } from '../lib/notificationOrigins'
import { useCallback, useEffect, useRef, useState, type ChangeEvent } from 'react'
import { Upload } from 'lucide-react'
import { useKoma } from '../store/koma'
import { BrailleSpinner } from './BrailleSpinner'
import { Field } from './panels/form'
import { insideSkillProject } from './skillProject'
import { showToast } from '../lib/toast'

const actionBtn = 'flex items-center gap-1 rounded border border-koma-border px-2.5 py-1 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover disabled:cursor-not-allowed disabled:opacity-40'

const MAX_ZIP_BYTES = 64 * 1024 * 1024
let uploadSeq = 0

function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const result = String(reader.result ?? '')
      const comma = result.indexOf(',')
      resolve(comma >= 0 ? result.slice(comma + 1) : result)
    }
    reader.onerror = () => reject(reader.error ?? new Error('Failed to read ZIP file'))
    reader.readAsDataURL(file)
  })
}

function skillNameFromFilename(filename: string): string {
  return filename
    .replace(/\.zip$/i, '')
    .toLocaleLowerCase()
    .replace(/[^a-z0-9_-]+/g, '-')
    .replace(/^-+|-+$/g, '')
}

export default function UploadSkillTab() {
  const req = useKoma((state) => state.req)
  const closeTab = useKoma((state) => state.closeTab)
  const epoch = useKoma((state) => state.skillSessionEpoch)
  const sessionId = useKoma((state) => state.session.id)
  const workdirs = useKoma((state) => state.settingsValues?.workdir)
  const activeRoot = useKoma((state) => state.coding.activeRoot)
  const insideProject = insideSkillProject(sessionId, workdirs, activeRoot)
  const lastOp = useKoma((state) => state.skillLastOp)
  const [scope, setScope] = useState<'global' | 'project'>('global')
  const [requestId, setRequestId] = useState<string | null>(null)
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [localError, setLocalError] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  const result = requestId && lastOp?.requestId === requestId ? lastOp : null
  const success = result?.outcomes.some((outcome) => outcome.status === 'success') ?? false
  const backendError = result?.outcomes.find((outcome) => outcome.status === 'failed')?.error
  const busy = Boolean(requestId && !result)

  useEffect(() => {
    if (!insideProject && scope === 'project') setScope('global')
  }, [insideProject, scope])

  useEffect(() => {
    if (!success) return
    const timer = window.setTimeout(() => closeTab('upload-skill'), 1200)
    return () => window.clearTimeout(timer)
  }, [closeTab, success])

  const chooseFile = useCallback(() => inputRef.current?.click(), [])
  const onFile = useCallback(async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (!file || !insideSkillProject(useKoma.getState().session.id, useKoma.getState().settingsValues?.workdir, useKoma.getState().coding.activeRoot)) return
    setLocalError(null)
    setRequestId(null)
    setSelectedFile(file.name)
    if (!file.name.toLocaleLowerCase().endsWith('.zip')) {
      setLocalError('Choose a .zip skill package.')
      return
    }
    if (file.size > MAX_ZIP_BYTES) {
      setLocalError('ZIP package exceeds the 64 MiB upload limit.')
      return
    }
    const name = skillNameFromFilename(file.name)
    if (!/^[a-z0-9][a-z0-9_-]*$/.test(name)) {
      setLocalError('The ZIP filename does not produce a valid skill name.')
      return
    }
    try {
      const dataB64 = await readFileAsBase64(file)
      const id = `install-zip-${++uploadSeq}`
      setRequestId(id)
      req({ r: 'InstallSkillZip', target: scope, name, dataB64, requestId: id, sessionEpoch: epoch, tabId: 'upload-skill' })
    } catch (error) {
      setLocalError(error instanceof Error ? error.message : 'Failed to read ZIP file')
    }
  }, [epoch, req, scope])

  const error = localError ?? backendError
  useEffect(() => {
    if (success) showToast('Skill installed successfully.', 'success', notificationOrigin(requestId ?? undefined))
  }, [success, requestId])
  useEffect(() => {
    if (error) showToast(error, 'error', notificationOrigin(requestId ?? undefined))
  }, [error])
  return (
    <div className="flex h-full min-w-0 flex-col bg-koma-bg text-koma-fg">
      <div className="min-h-0 flex-1 overflow-auto">
        <div className="mx-auto flex max-w-3xl flex-col gap-1 px-8 py-6">
          <div className="mb-4 border-b border-koma-border pb-2">
            <h2 className="text-[15px] font-semibold text-koma-fg">Upload skill (.zip)</h2>
            <p className="mt-0.5 text-[12px] text-koma-fg opacity-45">Install a packaged skill into Global or Project scope.</p>
          </div>

          {!success && <>
            <Field label="Target scope">
              <div className="flex gap-1">
                {(['global', 'project'] as const).map((value) => (
                  <button
                    type="button"
                    key={value}
                    disabled={busy || !insideProject}
                    title={!insideProject ? (value === 'project' ? 'Open a project to install a Project skill' : 'Open a project to add a skill') : undefined}
                    onClick={() => setScope(value)}
                    aria-pressed={scope === value}
                    className={`rounded border px-2.5 py-1 text-[12px] text-koma-fg transition-colors hover:bg-koma-hover disabled:cursor-not-allowed disabled:opacity-40 ${scope === value ? 'border-koma-accent bg-koma-accent/15' : 'border-koma-border'}`}
                  >
                    {value === 'global' ? 'Global' : 'Project'}
                  </button>
                ))}
              </div>
              {!insideProject && <span className="text-[11px] text-koma-fg opacity-45">Open a project to add a skill. Project scope stays unavailable until then.</span>}
            </Field>
            <div className="px-3 py-1.5">
              <button type="button" onClick={chooseFile} disabled={busy || !insideProject} title={insideProject ? undefined : 'Open a project to add a skill'} className="flex min-h-28 w-full flex-col items-center justify-center gap-2 rounded border border-dashed border-koma-border px-3 py-5 text-[12px] text-koma-fg transition-colors hover:border-koma-accent hover:bg-koma-hover disabled:cursor-not-allowed disabled:opacity-40">
                {busy ? <BrailleSpinner size={16} /> : <Upload size={18} className="text-koma-accent" />}
                {busy ? 'Installing…' : selectedFile ? `Choose another ZIP (${selectedFile})` : 'Choose .zip package'}
              </button>
              <input ref={inputRef} type="file" accept=".zip,application/zip" className="hidden" onChange={onFile} disabled={busy || !insideProject} />
              <p className="mt-2 text-[11px] text-koma-fg opacity-45">ZIP up to 64 MiB. Must contain a valid SKILL.md.</p>
            </div>
            {/* Keep these limits aligned with src-agent/src/model/skill/persistence.rs and its safety tests. */}
            <details className="mx-3 rounded border border-koma-border px-3 py-2 text-koma-fg">
              <summary className="cursor-pointer text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-50">Package requirements</summary>
              <ul className="mt-2 list-disc space-y-1 pl-4 text-[11px] leading-relaxed text-koma-fg opacity-45">
                <li>Maximum 1,000 files and folder depth 16.</li>
                <li>Maximum 8 MiB per companion file.</li>
                <li>Maximum 2 MiB for SKILL.md.</li>
                <li>Maximum 64 MiB compressed and 64 MiB uncompressed total.</li>
                <li>No symlinks, special files, path traversal, collisions, or malformed SKILL.md.</li>
              </ul>
            </details>
          </>}
        </div>
      </div>
      <footer className="flex flex-none flex-wrap items-center justify-end gap-2 border-t border-koma-border px-4 py-2.5">
        <button type="button" onClick={() => closeTab('upload-skill')} disabled={busy} className={actionBtn}>Cancel</button>
      </footer>
    </div>
  )
}
