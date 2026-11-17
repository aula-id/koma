import { useId, useState } from 'react'
import { TextArea, TextInput, Toggle, Segmented } from '../form'
import { DetailHeader, FormActions } from '../helpers'
import { FormGroup, FormRow } from './McpGroup'
import {
  badSeconds,
  envSummary,
  headersSummary,
  limitsHasValues,
  limitsSummary,
  toolsSummary,
  linesToPairs,
  splitList,
  type McpDraft,
} from './mcpForm'

const VAR_HINT = 'Use ${VAR} to read a value from the environment.'

type Props = {
  draft: McpDraft
  isNew: boolean
  onChange: (patch: Partial<McpDraft>) => void
  onSave: () => void
  onCancel: () => void
}

export function McpEditView({ draft, isNew, onChange, onSave, onCancel }: Props) {
  const uid = useId()
  const id = (n: string) => `${uid}-${n}`
  // Initial state only: groups holding values open when editing; the user's toggles win after.
  const [envOpen, setEnvOpen] = useState(() => linesToPairs(draft.envText, '=').length > 0)
  const [headersOpen, setHeadersOpen] = useState(() => linesToPairs(draft.headersText, ':').length > 0)
  const [limitsOpen, setLimitsOpen] = useState(() => limitsHasValues(draft))
  const [toolsOpen, setToolsOpen] = useState(
    () => splitList(draft.allowedText).length + splitList(draft.disabledText).length > 0,
  )
  const startupBad = badSeconds(draft.startupSec)
  const toolBad = badSeconds(draft.toolSec)
  const stdio = draft.transport === 'stdio'
  return (
    <>
      <DetailHeader onBack={onCancel} title={isNew ? 'Add server' : 'Edit server'} />
      <div className="min-h-0 min-w-0 flex-1 overflow-y-auto overflow-x-hidden">
        <div className="flex min-w-0 flex-col gap-4 px-3 py-4">
          <FormRow label="Name" htmlFor={id('name')}>
            <TextInput
              id={id('name')}
              value={draft.name}
              autoFocus
              placeholder="e.g. filesystem"
              onChange={(e) => onChange({ name: e.target.value })}
            />
          </FormRow>
          <FormRow
            label="How does it run?"
            hint={
              stdio
                ? 'stdio: starts a program on this computer and talks to it directly.'
                : 'http: connects to a server that is already running, by URL.'
            }
          >
            <Segmented
              value={draft.transport}
              onChange={(v) => onChange({ transport: v })}
              options={[
                { value: 'stdio', label: 'Run a command' },
                { value: 'http', label: 'Connect to URL' },
              ]}
            />
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
                hint="One argument per line, in order. Do not quote them."
              >
                <TextArea
                  id={id('args')}
                  value={draft.argsText}
                  rows={3}
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
          <div className="flex items-center justify-between gap-3">
            <span className="flex min-w-0 flex-col">
              <span id={id('enabled')} className="text-[11px] font-semibold text-koma-fg opacity-75">
                Enabled
              </span>
              <span className="text-[11px] text-koma-fg opacity-55">Off keeps the server saved but unused.</span>
            </span>
            <Toggle labelledBy={id('enabled')} on={draft.enabled} onChange={(v) => onChange({ enabled: v })} />
          </div>
        </div>

        <div className="flex min-w-0 flex-col gap-2 border-t border-koma-border px-3 py-4">
          <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-50">
            Advanced
          </span>
          {stdio && (
            <FormGroup
              title="Environment variables"
              summary={envSummary(draft)}
              open={envOpen}
              onToggle={() => setEnvOpen((o) => !o)}
            >
              <FormRow
                label="Variables"
                htmlFor={id('env')}
                hint={`One KEY=value per line. ${VAR_HINT}`}
              >
                <TextArea
                  id={id('env')}
                  value={draft.envText}
                  rows={3}
                  placeholder="GITHUB_TOKEN=${GITHUB_TOKEN}"
                  onChange={(e) => onChange({ envText: e.target.value })}
                />
              </FormRow>
            </FormGroup>
          )}
          {!stdio && (
            <FormGroup
              title="Headers"
              summary={headersSummary(draft)}
              open={headersOpen}
              onToggle={() => setHeadersOpen((o) => !o)}
            >
              <FormRow
                label="Request headers"
                htmlFor={id('headers')}
                hint={`One Name: value per line. ${VAR_HINT} Saved values are hidden; leave a value as is to keep it.`}
              >
                <TextArea
                  id={id('headers')}
                  value={draft.headersText}
                  rows={3}
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
              <FormRow
                label="Working directory"
                htmlFor={id('cwd')}
                hint="Leave empty to use the session folder."
              >
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
              hint="In seconds. Empty uses the default (20)."
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
              hint="In seconds. Empty uses the default (60)."
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
              hint="Comma separated; * matches any text, e.g. read_*. Empty allows all."
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
              hint="Comma separated; * matches any text. Wins over the allowed list."
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
      <FormActions onCancel={onCancel} onSave={onSave} saveDisabled={!draft.name.trim()} />
    </>
  )
}
