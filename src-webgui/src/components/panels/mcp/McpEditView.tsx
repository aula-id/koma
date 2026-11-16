import { Field, TextArea, TextInput, Toggle, Segmented } from '../form'
import { DetailHeader, FormActions } from '../helpers'
import type { McpDraft } from './mcpForm'

const VAR_HINT = 'Use ${VAR} to read a value from the environment.'

type Props = {
  draft: McpDraft
  isNew: boolean
  onChange: (patch: Partial<McpDraft>) => void
  onSave: () => void
  onCancel: () => void
}

export function McpEditView({ draft, isNew, onChange, onSave, onCancel }: Props) {
  return (
    <>
      <DetailHeader onBack={onCancel} title={isNew ? 'Add server' : 'Edit server'} />
      <div className="flex-1 overflow-auto py-1">
        <Field label="Name">
          <TextInput
            value={draft.name}
            autoFocus
            placeholder="e.g. filesystem"
            onChange={(e) => onChange({ name: e.target.value })}
          />
        </Field>
        <div className="flex items-center justify-between px-3 py-1.5">
          <span className="text-[10px] font-semibold uppercase tracking-wider text-koma-fg opacity-50">
            Enabled
          </span>
          <Toggle on={draft.enabled} onChange={(v) => onChange({ enabled: v })} />
        </div>
        <Field label="Transport">
          <Segmented
            value={draft.transport}
            onChange={(v) => onChange({ transport: v })}
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
                onChange={(e) => onChange({ command: e.target.value })}
              />
            </Field>
            <Field label="Arguments (one per line)">
              <TextArea
                value={draft.argsText}
                placeholder={'-y\n@modelcontextprotocol/server-filesystem\n.'}
                onChange={(e) => onChange({ argsText: e.target.value })}
              />
            </Field>
            <Field label="Environment (KEY=value per line)">
              <TextArea
                value={draft.envText}
                placeholder="GITHUB_TOKEN=${GITHUB_TOKEN}"
                onChange={(e) => onChange({ envText: e.target.value })}
              />
              <span className="text-[10px] text-koma-fg opacity-40">{VAR_HINT}</span>
            </Field>
            <Field label="Working directory">
              <TextInput
                value={draft.cwd}
                placeholder="defaults to the session folder"
                onChange={(e) => onChange({ cwd: e.target.value })}
              />
            </Field>
          </>
        ) : (
          <>
            <Field label="URL">
              <TextInput
                value={draft.url}
                placeholder="https://…"
                onChange={(e) => onChange({ url: e.target.value })}
              />
            </Field>
            <Field label="Headers (Name: value per line)">
              <TextArea
                value={draft.headersText}
                placeholder="Authorization: Bearer ${TOKEN}"
                onChange={(e) => onChange({ headersText: e.target.value })}
              />
              <span className="text-[10px] text-koma-fg opacity-40">
                {VAR_HINT} Saved values are hidden; leave a value as is to keep it.
              </span>
            </Field>
          </>
        )}
        <Field label="Startup timeout (seconds)">
          <TextInput
            value={draft.startupSec}
            inputMode="decimal"
            placeholder="20"
            onChange={(e) => onChange({ startupSec: e.target.value })}
          />
        </Field>
        <Field label="Tool timeout (seconds)">
          <TextInput
            value={draft.toolSec}
            inputMode="decimal"
            placeholder="60"
            onChange={(e) => onChange({ toolSec: e.target.value })}
          />
        </Field>
        <Field label="Allowed tools">
          <TextInput
            value={draft.allowedText}
            placeholder="comma separated, * wildcard (empty = all)"
            onChange={(e) => onChange({ allowedText: e.target.value })}
          />
        </Field>
        <Field label="Disabled tools">
          <TextInput
            value={draft.disabledText}
            placeholder="comma separated, * wildcard"
            onChange={(e) => onChange({ disabledText: e.target.value })}
          />
        </Field>
      </div>
      <FormActions onCancel={onCancel} onSave={onSave} saveDisabled={!draft.name.trim()} />
    </>
  )
}
