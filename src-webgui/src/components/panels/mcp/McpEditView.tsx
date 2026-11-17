import { useId, useState } from 'react'
import { VectorSquare } from 'lucide-react'
import { TextArea, TextInput, Toggle, Segmented } from '../form'
import { HelpTip } from '../helpers'
import type { McpServer } from '../../../types/config'
import { FormGroup, FormRow, McpFooter } from './McpGroup'
import {
  badSeconds,
  envSummary,
  headersSummary,
  limitsHasValues,
  limitsSummary,
  serverStatus,
  toolsSummary,
  linesToPairs,
  splitList,
  type McpDraft,
} from './mcpForm'

const VarNote = () => (
  <>
    Use <code>{'${VAR}'}</code> to read a value from the environment.
  </>
)

const TONE: Record<string, string> = {
  ok: 'bg-emerald-500',
  warn: 'bg-amber-500',
  wait: 'bg-amber-500/50',
  off: 'bg-koma-fg/25',
}

const MONO_AREA = 'whitespace-pre overflow-x-auto leading-relaxed'

type Props = {
  draft: McpDraft
  isNew: boolean
  /** The saved server, for the header's status line. Absent on a new server. */
  server?: McpServer
  onChange: (patch: Partial<McpDraft>) => void
  onSave: () => void
  onCancel: () => void
}

/** Full-height editor page: header, scrolling body (centred column), sticky Save/Cancel bar. */
export function McpEditView({ draft, isNew, server, onChange, onSave, onCancel }: Props) {
  const uid = useId()
  const id = (n: string) => `${uid}-${n}`
  // Initial state only: groups holding values open; the user's toggles win after.
  const [envOpen, setEnvOpen] = useState(() => linesToPairs(draft.envText, '=').length > 0)
  const [headersOpen, setHeadersOpen] = useState(() => linesToPairs(draft.headersText, ':').length > 0)
  const [limitsOpen, setLimitsOpen] = useState(() => limitsHasValues(draft))
  const [toolsOpen, setToolsOpen] = useState(
    () => splitList(draft.allowedText).length + splitList(draft.disabledText).length > 0,
  )
  const startupBad = badSeconds(draft.startupSec)
  const toolBad = badSeconds(draft.toolSec)
  const stdio = draft.transport === 'stdio'
  const status = server ? serverStatus(server) : null
  return (
    <div className="flex h-full min-h-0 w-full min-w-0 flex-col bg-koma-bg text-koma-fg">
      <div className="@container min-h-0 min-w-0 flex-1 overflow-y-auto overflow-x-hidden">
        <div className="mx-auto flex w-full min-w-0 max-w-3xl flex-col gap-6 px-6 py-6 @lg:py-8">
          <header className="flex min-w-0 flex-col gap-2 border-b border-koma-border pb-4">
            <div className="flex min-w-0 items-center gap-2.5">
              <VectorSquare size={20} strokeWidth={1.75} className="flex-none opacity-70" />
              <h1 className="min-w-0 break-words text-[18px] font-semibold leading-tight">
                {isNew ? 'New MCP server' : draft.name.trim() || '(unnamed)'}
              </h1>
              {!isNew && (
                <span className="flex-none rounded bg-koma-head px-1.5 py-px text-[10px] uppercase tracking-wide opacity-70">
                  {draft.transport}
                </span>
              )}
            </div>
            {status && (
              <div className="flex min-w-0 items-center gap-2 text-[12px]" data-testid="mcp-status">
                <span className={`h-2 w-2 flex-none rounded-full ${TONE[status.tone]}`} />
                <span className="min-w-0 break-words opacity-80">{status.label}</span>
              </div>
            )}
            <p className="flex flex-wrap items-center gap-1 text-[12px] leading-relaxed opacity-60">
              MCP servers connect Koma to other tools and data, like files, a database or a web service.
              <HelpTip label="MCP servers">
                MCP is the Model Context Protocol. Each server you add gives the agent more tools. Tools appear once the server is connected.
              </HelpTip>
            </p>
          </header>

          <section aria-label="Essentials" className="flex min-w-0 flex-col gap-5 rounded-lg border border-koma-border bg-koma-panel p-4 @lg:p-5">
            <FormRow label="Name" htmlFor={id('name')}>
              <TextInput
                id={id('name')}
                value={draft.name}
                autoFocus={isNew}
                placeholder="e.g. filesystem"
                onChange={(e) => onChange({ name: e.target.value })}
              />
            </FormRow>
            <FormRow
              label="How does it run?"
              help={
                <>
                  <b>Run a command</b> (stdio) starts a program on this computer and talks to it directly.{' '}
                  <b>Connect to URL</b> (http) connects to a server that is already running.
                </>
              }
            >
              <div className="max-w-sm">
                <Segmented
                  value={draft.transport}
                  onChange={(v) => onChange({ transport: v })}
                  options={[
                    { value: 'stdio', label: 'Run a command' },
                    { value: 'http', label: 'Connect to URL' },
                  ]}
                />
              </div>
            </FormRow>
            {stdio ? (
              <>
                <FormRow label="Command" htmlFor={id('command')}>
                  <TextInput
                    id={id('command')}
                    value={draft.command}
                    placeholder="npx"
                    onChange={(e) => onChange({ command: e.target.value })}
                  />
                </FormRow>
                <FormRow
                  label="Arguments"
                  htmlFor={id('args')}
                  help="One argument per line, in order. Do not quote them."
                >
                  <TextArea
                    id={id('args')}
                    className={MONO_AREA}
                    wrap="off"
                    spellCheck={false}
                    value={draft.argsText}
                    rows={5}
                    placeholder={'-y\n@modelcontextprotocol/server-filesystem\n.'}
                    onChange={(e) => onChange({ argsText: e.target.value })}
                  />
                </FormRow>
              </>
            ) : (
              <FormRow label="URL" htmlFor={id('url')}>
                <TextInput
                  id={id('url')}
                  value={draft.url}
                  placeholder="https://…"
                  onChange={(e) => onChange({ url: e.target.value })}
                />
              </FormRow>
            )}
            <div className="flex items-center justify-between gap-3 border-t border-koma-border pt-4">
              <span className="flex min-w-0 items-center gap-1">
                <span id={id('enabled')} className="text-[11px] font-semibold opacity-75">
                  Enabled
                </span>
                <HelpTip label="Enabled">Off keeps the server saved but unused.</HelpTip>
              </span>
              <Toggle labelledBy={id('enabled')} on={draft.enabled} onChange={(v) => onChange({ enabled: v })} />
            </div>
          </section>

          <div className="flex min-w-0 flex-col gap-3">
            <h2 className="text-[10px] font-semibold uppercase tracking-wider opacity-50">Advanced</h2>
            <div className="grid min-w-0 grid-cols-1 items-start gap-3 @3xl:grid-cols-2">
              {stdio && (
                <FormGroup
                  title="Environment variables"
                  help={
                    <>
                      One <code>KEY=value</code> per line. <VarNote />
                    </>
                  }
                  summary={envSummary(draft)}
                  open={envOpen}
                  onToggle={() => setEnvOpen((o) => !o)}
                >
                  <FormRow label="Variables" htmlFor={id('env')}>
                    <TextArea
                      id={id('env')}
                      className={MONO_AREA}
                      wrap="off"
                      spellCheck={false}
                      value={draft.envText}
                      rows={5}
                      placeholder="GITHUB_TOKEN=${GITHUB_TOKEN}"
                      onChange={(e) => onChange({ envText: e.target.value })}
                    />
                  </FormRow>
                </FormGroup>
              )}
              {!stdio && (
                <FormGroup
                  title="Headers"
                  help={
                    <>
                      One <code>Name: value</code> per line. <VarNote /> Saved values are hidden; leave a value as is to keep it.
                    </>
                  }
                  summary={headersSummary(draft)}
                  open={headersOpen}
                  onToggle={() => setHeadersOpen((o) => !o)}
                >
                  <FormRow label="Request headers" htmlFor={id('headers')}>
                    <TextArea
                      id={id('headers')}
                      className={MONO_AREA}
                      wrap="off"
                      spellCheck={false}
                      value={draft.headersText}
                      rows={5}
                      placeholder="Authorization: Bearer ${TOKEN}"
                      onChange={(e) => onChange({ headersText: e.target.value })}
                    />
                  </FormRow>
                </FormGroup>
              )}
              <FormGroup
                title={stdio ? 'Working directory and timeouts' : 'Timeouts'}
                summary={limitsSummary(draft)}
                open={limitsOpen || startupBad || toolBad}
                onToggle={() => setLimitsOpen((o) => !o)}
              >
                {stdio && (
                  <FormRow label="Working directory" htmlFor={id('cwd')} help="Leave empty to use the session folder.">
                    <TextInput
                      id={id('cwd')}
                      value={draft.cwd}
                      placeholder="/path/to/folder"
                      onChange={(e) => onChange({ cwd: e.target.value })}
                    />
                  </FormRow>
                )}
                <FormRow
                  label="Startup timeout"
                  htmlFor={id('startup')}
                  error={startupBad ? 'Enter a number above 0, or clear it to use the default.' : undefined}
                  help="In seconds. Empty uses the default (20)."
                >
                  <TextInput
                    id={id('startup')}
                    value={draft.startupSec}
                    inputMode="decimal"
                    placeholder="20"
                    onChange={(e) => onChange({ startupSec: e.target.value })}
                  />
                </FormRow>
                <FormRow
                  label="Tool call timeout"
                  htmlFor={id('tool')}
                  error={toolBad ? 'Enter a number above 0, or clear it to use the default.' : undefined}
                  help="In seconds. Empty uses the default (60)."
                >
                  <TextInput
                    id={id('tool')}
                    value={draft.toolSec}
                    inputMode="decimal"
                    placeholder="60"
                    onChange={(e) => onChange({ toolSec: e.target.value })}
                  />
                </FormRow>
              </FormGroup>
              <FormGroup
                title="Tool access"
                summary={toolsSummary(draft)}
                open={toolsOpen}
                onToggle={() => setToolsOpen((o) => !o)}
              >
                <FormRow
                  label="Allowed tools"
                  htmlFor={id('allowed')}
                  help={
                    <>
                      Comma separated; <code>*</code> matches any text, e.g. <code>read_*</code>. Empty allows all.
                    </>
                  }
                >
                  <TextInput
                    id={id('allowed')}
                    value={draft.allowedText}
                    placeholder="read_*, list_*"
                    onChange={(e) => onChange({ allowedText: e.target.value })}
                  />
                </FormRow>
                <FormRow
                  label="Disabled tools"
                  htmlFor={id('disabled')}
                  help="Comma separated; * matches any text. Wins over the allowed list."
                >
                  <TextInput
                    id={id('disabled')}
                    value={draft.disabledText}
                    placeholder="delete_*"
                    onChange={(e) => onChange({ disabledText: e.target.value })}
                  />
                </FormRow>
              </FormGroup>
            </div>
          </div>
        </div>
      </div>
      <McpFooter onCancel={onCancel} onSave={onSave} saveDisabled={!draft.name.trim()} />
    </div>
  )
}
