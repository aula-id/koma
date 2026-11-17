import { Plug, Plus, FileJson, Pencil, Trash2, Check, X, RotateCw } from 'lucide-react'
import type { McpProject, McpServer } from '../../../types/config'
import { EmptyState } from '../helpers'
import { mcpEditTabId } from '../../../store/actions/mcp'

type Props = {
  servers: McpServer[]
  project?: McpProject
  armed: string | null
  /** Id of the active editor tab; the row whose tab it is gets highlighted. */
  activeTabId?: string
  onAdd: () => void
  onImport: () => void
  onEdit: (s: McpServer) => void
  onArm: (id: string) => void
  onDisarm: () => void
  onConfirm: (id: string) => void
  onToggleEnable: (id: string, enabled: boolean) => void
  onRestart: (id: string) => void
  onApprove: (id: string, approved: boolean) => void
}

export function McpListView({
  servers: all,
  project,
  armed,
  activeTabId,
  onAdd,
  onImport,
  onEdit,
  onArm,
  onDisarm,
  onConfirm,
  onToggleEnable,
  onRestart,
  onApprove,
}: Props) {
  const servers = all.filter((s) => s.scope !== 'project')
  const projectServers = all.filter((s) => s.scope === 'project')
  const row = (s: McpServer) => {
    const isProject = s.scope === 'project'
    const approved = !isProject || s.approval === 'approved'
    return (
      <div
        key={s.id}
        aria-current={activeTabId === mcpEditTabId(s.id) ? 'true' : undefined}
        className={`group flex min-h-[38px] items-center gap-2.5 px-3 py-1.5 hover:bg-koma-hover ${
          activeTabId === mcpEditTabId(s.id) ? 'bg-koma-hover' : ''
        }`}
      >
        {armed === s.id ? (
          <div className="flex flex-1 items-center gap-2">
            <span className="flex-1 truncate text-[12px] text-koma-fg">
              Delete "{s.name || 'server'}"?
            </span>
            <button
              onClick={() => onConfirm(s.id)}
              aria-label="Confirm delete"
              className="flex h-5 w-5 items-center justify-center rounded text-koma-fg opacity-70 transition-colors hover:text-emerald-500 hover:opacity-100"
            >
              <Check size={14} />
            </button>
            <button
              onClick={onDisarm}
              aria-label="Cancel delete"
              className="flex h-5 w-5 items-center justify-center rounded text-koma-fg opacity-70 transition-colors hover:text-red-500 hover:opacity-100"
            >
              <X size={14} />
            </button>
          </div>
        ) : (
          <>
            <button
              onClick={() =>
                isProject ? onApprove(s.id, !approved) : onToggleEnable(s.id, !s.enabled)
              }
              aria-label={
                isProject
                  ? approved
                    ? 'Revoke approval'
                    : 'Approve'
                  : s.enabled
                    ? 'Disable'
                    : 'Enable'
              }
              title={
                !approved
                  ? s.approval === 'changed'
                    ? 'Changed since approval'
                    : 'Needs approval'
                  : !s.enabled
                    ? 'Disabled'
                    : s.error
                      ? `Error: ${s.error}`
                      : s.connected
                        ? s.toolCount > 0
                          ? `Connected (${s.toolCount} tools)`
                          : 'Connected (0 tools)'
                        : 'Checking…'
              }
              className={`h-2 w-2 flex-none rounded-full transition-opacity hover:opacity-70 ${
                !approved || !s.enabled
                  ? 'bg-koma-fg/25'
                  : s.error
                    ? 'bg-amber-500'
                    : s.connected
                      ? 'bg-emerald-500'
                      : 'bg-amber-500/50'
              }`}
            />
            <button
              onClick={() => (isProject ? undefined : onEdit(s))}
              className="min-w-0 flex-1 text-left"
            >
              <div className="flex items-center gap-2">
                <span
                  className={`min-w-0 truncate text-[13px] text-koma-fg ${s.enabled ? '' : 'opacity-50'}`}
                >
                  {s.name || '(unnamed)'}
                </span>
                <span className="flex-none rounded bg-koma-head px-1 py-px text-[10px] uppercase tracking-wide text-koma-fg opacity-60">
                  {s.transport}
                </span>
              </div>
              {isProject && !approved && (
                <div className="mt-0.5 truncate text-[11px] text-amber-500/80">
                  {s.approval === 'changed'
                    ? 'Changed since approval'
                    : 'Not started until you approve'}
                </div>
              )}
              {s.overridden && (
                <div className="mt-0.5 truncate text-[11px] text-koma-fg opacity-50">
                  Overridden by .mcp.json
                </div>
              )}
              {s.enabled && s.error && (
                <div className="mt-0.5 truncate text-[11px] text-amber-500/80" title={s.error}>
                  {s.error}
                </div>
              )}
            </button>
            {isProject && !approved && (
              <button
                onClick={() => onApprove(s.id, true)}
                className="flex-none rounded border border-koma-border px-1.5 py-px text-[11px] text-koma-fg opacity-80 transition hover:bg-koma-head hover:opacity-100"
              >
                Approve
              </button>
            )}
            {/* Idle: max-w-0 so actions take no flex width (no permanent empty gutter).
                    Hover: expand and show. opacity alone would still reserve space. */}
            <div className="flex max-w-0 flex-none items-center overflow-hidden opacity-0 transition-[max-width,opacity] duration-100 group-hover:max-w-[66px] group-hover:opacity-100 group-has-[:focus-visible]:max-w-[66px] group-has-[:focus-visible]:opacity-100">
              {s.enabled && (
                <button
                  onClick={() => onRestart(s.id)}
                  aria-label="Restart"
                  title="Restart"
                  className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 transition hover:!opacity-100"
                >
                  <RotateCw size={13} />
                </button>
              )}
              {!isProject && (
                <>
                  <button
                    onClick={() => onEdit(s)}
                    aria-label="Edit"
                    className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 transition hover:!opacity-100"
                  >
                    <Pencil size={13} />
                  </button>
                  <button
                    onClick={() => onArm(s.id)}
                    aria-label="Delete"
                    className="flex h-5 w-5 flex-none items-center justify-center rounded text-koma-fg opacity-70 transition hover:!text-red-500 hover:!opacity-100"
                  >
                    <Trash2 size={13} />
                  </button>
                </>
              )}
            </div>
          </>
        )}
      </div>
    )
  }
  return (
    <>
      <div className="flex-1 overflow-auto py-1">
        {servers.length === 0 && projectServers.length === 0 && !project && (
          <EmptyState icon={Plug} title="No MCP servers yet" hint="Add one with the Add server button below.">
            MCP servers connect Koma to other tools and data, like files, a database or a web service.
          </EmptyState>
        )}
        {servers.map(row)}
        {project && (
          <div
            className="mt-2 px-3 pb-1 pt-2 text-[10px] uppercase tracking-wide text-koma-fg opacity-50"
            title={project.file}
          >
            Project (.mcp.json)
          </div>
        )}
        {project?.error && (
          <div className="px-3 pb-1 text-[11px] text-amber-500/80" title={project.error}>
            {project.error}
          </div>
        )}
        {projectServers.map(row)}
      </div>
      <div className="flex flex-none gap-2 border-t border-koma-border p-2">
        <button
          onClick={onAdd}
          className="flex flex-1 items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          <Plus size={14} /> Add server
        </button>
        <button
          onClick={onImport}
          className="flex flex-1 items-center justify-center gap-1.5 rounded border border-koma-border py-1.5 text-[12px] text-koma-fg opacity-70 transition-colors hover:bg-koma-hover hover:opacity-100"
        >
          <FileJson size={14} /> Import JSON
        </button>
      </div>
    </>
  )
}
