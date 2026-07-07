import { useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { Plus, Pencil, Trash2, Check, X, ChevronLeft } from 'lucide-react'
import { Field, TextInput, Toggle, Segmented } from './form'

type Transport = 'stdio' | 'http'

type Server = {
  id: string
  name: string
  enabled: boolean
  transport: Transport
  command: string
  args: string
  env: string
  url: string
}

let seq = 0
function blankServer(): Server {
  seq += 1
  return {
    id: `srv-${seq}`,
    name: '',
    enabled: true,
    transport: 'stdio',
    command: '',
    args: '',
    env: '',
    url: '',
  }
}

const SLIDE = { type: 'spring', stiffness: 520, damping: 42 } as const

// Design reference: master -> detail slide, inline form, inline arm-delete — no
// popups. State is local and starts EMPTY; Add fills it so the whole add/edit/
// remove loop is demoable without any backend.
export function McpPanel() {
  const [servers, setServers] = useState<Server[]>([])
  const [draft, setDraft] = useState<Server | null>(null)
  const [isNew, setIsNew] = useState(false)
  const [armed, setArmed] = useState<string | null>(null)

  const openAdd = () => {
    setDraft(blankServer())
    setIsNew(true)
  }
  const openEdit = (s: Server) => {
    setDraft({ ...s })
    setIsNew(false)
  }
  const patch = (p: Partial<Server>) => setDraft((d) => (d ? { ...d, ...p } : d))
  const cancel = () => setDraft(null)
  const save = () => {
    if (!draft || !draft.name.trim()) return
    setServers((list) =>
      isNew ? [...list, draft] : list.map((s) => (s.id === draft.id ? draft : s)),
    )
    setDraft(null)
  }
  const remove = (id: string) => {
    setServers((list) => list.filter((s) => s.id !== id))
    setArmed(null)
  }

  return (
    <div className="relative h-full overflow-hidden">
      <AnimatePresence initial={false}>
        {draft ? (
          <motion.div
            key="form"
            initial={{ x: '100%' }}
            animate={{ x: 0 }}
            exit={{ x: '100%' }}
            transition={SLIDE}
            className="absolute inset-0 flex flex-col bg-koma-panel"
          >
            <div className="flex h-8 flex-none items-center gap-1 border-b border-koma-border px-2">
              <button
                onClick={cancel}
                aria-label="Back"
                className="flex h-6 w-6 items-center justify-center rounded text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
              >
                <ChevronLeft size={16} />
              </button>
              <span className="text-[12px] font-semibold text-koma-fg">
                {isNew ? 'Add server' : 'Edit server'}
              </span>
            </div>

            <div className="flex-1 overflow-auto py-1">
              <Field label="Name">
                <TextInput
                  value={draft.name}
                  autoFocus
                  placeholder="e.g. filesystem"
                  onChange={(e) => patch({ name: e.target.value })}
                />
              </Field>
              <div className="flex items-center justify-between px-3 py-1.5">
                <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-50">
                  Enabled
                </span>
                <Toggle on={draft.enabled} onChange={(v) => patch({ enabled: v })} />
              </div>
              <Field label="Transport">
                <Segmented
                  value={draft.transport}
                  onChange={(v) => patch({ transport: v })}
                  options={[
                    { value: 'stdio', label: 'stdio' },
                    { value: 'http', label: 'http' },
                  ]}
                />
              </Field>
              {draft.transport === 'stdio' ? (
                <>
                  <Field label="Command">
                    <TextInput
                      value={draft.command}
                      placeholder="npx"
                      onChange={(e) => patch({ command: e.target.value })}
                    />
                  </Field>
                  <Field label="Args">
                    <TextInput
                      value={draft.args}
                      placeholder="space separated"
                      onChange={(e) => patch({ args: e.target.value })}
                    />
                  </Field>
                  <Field label="Env">
                    <TextInput
                      value={draft.env}
                      placeholder="KEY=VAL, KEY2=VAL2"
                      onChange={(e) => patch({ env: e.target.value })}
                    />
                  </Field>
                </>
              ) : (
                <Field label="URL">
                  <TextInput
                    value={draft.url}
                    placeholder="https://…"
                    onChange={(e) => patch({ url: e.target.value })}
                  />
                </Field>
              )}
            </div>

            <div className="flex flex-none items-center justify-end gap-2 border-t border-koma-border px-3 py-2">
              <button
                onClick={cancel}
                className="rounded px-2.5 py-1 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
              >
                Cancel
              </button>
              <button
                onClick={save}
                disabled={!draft.name.trim()}
                className="rounded border border-koma-border px-2.5 py-1 text-[12px] text-koma-fg transition-colors enabled:hover:bg-koma-hover disabled:opacity-40"
              >
                Save
              </button>
            </div>
          </motion.div>
        ) : (
          <motion.div
            key="list"
            initial={{ x: '-25%', opacity: 0 }}
            animate={{ x: 0, opacity: 1 }}
            exit={{ x: '-25%', opacity: 0 }}
            transition={SLIDE}
            className="absolute inset-0 flex flex-col bg-koma-panel"
          >
            <div className="flex-1 overflow-auto py-1">
              {servers.length === 0 && (
                <div className="px-3 py-6 text-center text-[12px] text-koma-fg opacity-35">
                  No MCP servers
                </div>
              )}
              {servers.map((s) => (
                <div
                  key={s.id}
                  className="group flex min-h-[38px] items-center gap-2.5 px-3 py-1.5 hover:bg-koma-hover"
                >
                  {armed === s.id ? (
                    <div className="flex flex-1 items-center gap-2">
                      <span className="flex-1 truncate text-[12px] text-koma-fg">
                        Delete “{s.name || 'server'}”?
                      </span>
                      <button
                        onClick={() => remove(s.id)}
                        aria-label="Confirm delete"
                        className="flex h-5 w-5 items-center justify-center rounded text-koma-fg opacity-70 transition-colors hover:text-emerald-500 hover:opacity-100"
                      >
                        <Check size={14} />
                      </button>
                      <button
                        onClick={() => setArmed(null)}
                        aria-label="Cancel delete"
                        className="flex h-5 w-5 items-center justify-center rounded text-koma-fg opacity-70 transition-colors hover:text-red-500 hover:opacity-100"
                      >
                        <X size={14} />
                      </button>
                    </div>
                  ) : (
                    <>
                      <span
                        className={`h-2 w-2 flex-none rounded-full ${s.enabled ? 'bg-emerald-500' : 'bg-koma-fg/25'}`}
                      />
                      <button onClick={() => openEdit(s)} className="min-w-0 flex-1 text-left">
                        <div className="flex items-center gap-2">
                          <span
                            className={`truncate text-[13px] text-koma-fg ${s.enabled ? '' : 'opacity-50'}`}
                          >
                            {s.name || '(unnamed)'}
                          </span>
                          <span className="flex-none rounded bg-koma-head px-1 py-px text-[10px] uppercase tracking-wide text-koma-fg opacity-60">
                            {s.transport}
                          </span>
                        </div>
                      </button>
                      <button
                        onClick={() => openEdit(s)}
                        aria-label="Edit"
                        className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-0 transition group-hover:opacity-60 hover:!opacity-100"
                      >
                        <Pencil size={13} />
                      </button>
                      <button
                        onClick={() => setArmed(s.id)}
                        aria-label="Delete"
                        className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-0 transition group-hover:opacity-60 hover:!text-red-500 hover:!opacity-100"
                      >
                        <Trash2 size={13} />
                      </button>
                    </>
                  )}
                </div>
              ))}
            </div>
            <div className="flex-none border-t border-koma-border p-2">
              <button
                onClick={openAdd}
                className="flex w-full items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
              >
                <Plus size={14} /> Add server
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}
