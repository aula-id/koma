import { useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { McpListView } from './mcp/McpListView'
import { McpEditView } from './mcp/McpEditView'

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

const SLIDE = { type: 'tween', duration: 0.22, ease: 'easeOut' } as const

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
            <McpEditView
              draft={draft}
              isNew={isNew}
              onChange={patch}
              onSave={save}
              onCancel={cancel}
            />
          </motion.div>
        ) : (
          <motion.div
            key="list"
            className="absolute inset-0 flex flex-col bg-koma-panel"
          >
            <McpListView
              servers={servers}
              armed={armed}
              onAdd={openAdd}
              onEdit={openEdit}
              onArm={(id) => setArmed(id)}
              onDisarm={() => setArmed(null)}
              onConfirm={remove}
            />
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}
