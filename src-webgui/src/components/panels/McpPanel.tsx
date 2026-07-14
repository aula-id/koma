import { useState, useEffect } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { McpListView } from './mcp/McpListView'
import { McpEditView } from './mcp/McpEditView'
import { useKoma } from '../../store/koma'
import type { McpServer } from '../../types/config'

let seq = 0
function blankServer(): McpServer {
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
    toolCount: 0,
  }
}

const SLIDE = { type: 'tween', duration: 0.22, ease: 'easeOut' } as const

// Design reference: master -> detail slide, inline form, inline arm-delete — no
// popups. `servers` is the authoritative config slice (pushed by the host);
// add/edit/delete/enable emit GuiReqs and wait for the host's Config push to
// reflect back, rather than mutating local state optimistically.
export function McpPanel() {
  const servers = useKoma((s) => s.config.mcp)
  const req = useKoma((s) => s.req)
  const refreshMcpStatus = useKoma((s) => s.refreshMcpStatus)
  const [draft, setDraft] = useState<McpServer | null>(null)
  const [isNew, setIsNew] = useState(false)
  const [armed, setArmed] = useState<string | null>(null)

  // Refresh live connection status each time the MCP sidebar is opened.
  useEffect(() => {
    refreshMcpStatus()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const openAdd = () => {
    setDraft(blankServer())
    setIsNew(true)
  }
  const openEdit = (s: McpServer) => {
    setDraft({ ...s })
    setIsNew(false)
  }
  const patch = (p: Partial<McpServer>) => setDraft((d) => (d ? { ...d, ...p } : d))
  const cancel = () => setDraft(null)
  const save = () => {
    if (!draft || !draft.name.trim()) return
    // Flat payload matching the daemon's GuiReq::SetMcpServer. `uuid` is the
    // daemon config uuid on edit; `null` for a new server (the client-side
    // `draft.id` is a synthetic placeholder, so the daemon mints a real uuid).
    req({
      r: 'SetMcpServer',
      uuid: isNew ? null : draft.id,
      name: draft.name,
      enabled: draft.enabled,
      transport: draft.transport,
      command: draft.command,
      args: draft.args,
      env: draft.env,
      url: draft.url,
    })
    setDraft(null)
  }
  const remove = (id: string) => {
    req({ r: 'DeleteMcpServer', uuid: id })
    setArmed(null)
  }
  const toggleEnable = (id: string, enabled: boolean) => {
    req({ r: 'EnableMcpServer', uuid: id, enabled })
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
              onToggleEnable={toggleEnable}
            />
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}
