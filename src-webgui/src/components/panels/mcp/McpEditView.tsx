import { Field, TextInput, Toggle, Segmented } from '../form'
import { DetailHeader, FormActions } from '../helpers'

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

type Props = {
  draft: Server
  isNew: boolean
  onChange: (patch: Partial<Server>) => void
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
            <Field label="Args">
              <TextInput
                value={draft.args}
                placeholder="space separated"
                onChange={(e) => onChange({ args: e.target.value })}
              />
            </Field>
            <Field label="Env">
              <TextInput
                value={draft.env}
                placeholder="KEY=VAL, KEY2=VAL2"
                onChange={(e) => onChange({ env: e.target.value })}
              />
            </Field>
          </>
        ) : (
          <Field label="URL">
            <TextInput
              value={draft.url}
              placeholder="https://…"
              onChange={(e) => onChange({ url: e.target.value })}
            />
          </Field>
        )}
      </div>
      <FormActions onCancel={onCancel} onSave={onSave} saveDisabled={!draft.name.trim()} />
    </>
  )
}
