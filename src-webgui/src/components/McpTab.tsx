import { useState } from 'react'
import { useKoma } from '../store/koma'
import type { Tab } from '../store/koma'
import { McpEditView } from './panels/mcp/McpEditView'
import { McpImportView } from './panels/mcp/McpImportView'
import { blankDraft, draftToRequest, toDraft, type McpDraft } from './panels/mcp/mcpForm'

type McpTabProps = { tab: Extract<Tab, { kind: 'mcp' }> }

/**
 * Editor tab for the MCP sidebar: a new server, one saved server, or the JSON import page.
 * The draft is local state, seeded once when the tab mounts, so later Config pushes (status
 * refreshes) never overwrite what is being typed. Save, Import and Cancel close the tab.
 */
export default function McpTab({ tab }: McpTabProps) {
  if (tab.mode === 'import') return <ImportBody tabId={tab.id} />
  return <EditBody tab={tab} />
}

function ImportBody({ tabId }: { tabId: string }) {
  const req = useKoma((s) => s.req)
  const closeTab = useKoma((s) => s.closeTab)
  return (
    <McpImportView
      onCancel={() => closeTab(tabId)}
      onImport={(text) => {
        req({ r: 'ImportMcpJson', text })
        closeTab(tabId)
      }}
    />
  )
}

function EditBody({ tab }: { tab: Extract<Tab, { kind: 'mcp'; mode: 'new' | 'edit' }> }) {
  const req = useKoma((s) => s.req)
  const closeTab = useKoma((s) => s.closeTab)
  const uuid = tab.mode === 'edit' ? tab.uuid : null
  const server = useKoma((s) => (uuid ? s.config.mcp.find((m) => m.id === uuid) : undefined))
  const [draft, setDraft] = useState<McpDraft | null>(() => (tab.mode === 'new' ? blankDraft() : server ? toDraft(server) : null))
  const isNew = tab.mode === 'new'
  const close = () => closeTab(tab.id)

  if (!draft) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 bg-koma-bg text-[13px] text-koma-fg">
        <span className="opacity-70">This MCP server no longer exists.</span>
        <button onClick={close} className="rounded border border-koma-border px-3 py-1 text-[12px] hover:bg-koma-hover">
          Close tab
        </button>
      </div>
    )
  }
  return (
    <McpEditView
      draft={draft}
      isNew={isNew}
      server={server}
      onChange={(p) => setDraft((d) => (d ? { ...d, ...p } : d))}
      onCancel={close}
      onSave={() => {
        if (!draft.name.trim()) return
        // `uuid` null makes the daemon mint one, so a saved new server has no tab id to
        // adopt: the tab closes and the new row appears in the list.
        req(draftToRequest(draft, isNew))
        close()
      }}
    />
  )
}
