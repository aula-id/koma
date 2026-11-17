import { useState } from 'react'
import { FileJson } from 'lucide-react'
import { EmbeddedCodeEditor } from '../../editor/EmbeddedCodeEditor'
import { HelpTip } from '../helpers'
import { FormRow, McpFooter } from './McpGroup'
import { MCP_JSON_SCHEMA, checkImportJson } from './mcpJson'

type Props = {
  onImport: (text: string) => void
  onCancel: () => void
}

const EXAMPLE = `{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "\${GITHUB_TOKEN}" }
    }
  }
}`

/** Full-height import page: paste area in a centred column, sticky Import/Cancel bar. */
export function McpImportView({ onImport, onCancel }: Props) {
  const [text, setText] = useState('')
  const [error, setError] = useState<string | null>(null)
  const submit = () => {
    if (!text.trim()) return
    const check = checkImportJson(text)
    if (!check.ok) {
      setError(check.error)
      return
    }
    setError(null)
    onImport(text)
  }
  return (
    <div className="flex h-full min-h-0 w-full min-w-0 flex-col bg-koma-bg text-koma-fg">
      <div className="@container min-h-0 min-w-0 flex-1 overflow-y-auto overflow-x-hidden">
        <div className="mx-auto flex w-full min-w-0 max-w-3xl flex-col gap-6 px-6 py-6 @lg:py-8">
          <header className="flex min-w-0 flex-col gap-2 border-b border-koma-border pb-4">
            <div className="flex min-w-0 items-center gap-2.5">
              <FileJson size={20} strokeWidth={1.75} className="flex-none opacity-70" />
              <h1 className="min-w-0 break-words text-[18px] font-semibold leading-tight">Import MCP servers</h1>
            </div>
            <p className="flex flex-wrap items-center gap-1 text-[12px] leading-relaxed opacity-60">
              Add several servers at once from a JSON snippet.
              <HelpTip label="Import MCP servers">
                Servers are added to your list. Tools appear once each server is connected.
              </HelpTip>
            </p>
          </header>
          <section className="flex min-w-0 flex-col gap-5 rounded-lg border border-koma-border bg-koma-panel p-4 @lg:p-5">
            <FormRow
              label="MCP servers JSON"
              error={error ?? undefined}
              help="Paste the mcpServers snippet from a server's README (Claude Code, Cursor or VS Code format). Ctrl/Cmd+Enter imports; Esc then Tab leaves the editor."
            >
              <EmbeddedCodeEditor
                testId="mcp-import-editor"
                ariaLabel="MCP servers JSON"
                language="json"
                jsonSchema={MCP_JSON_SCHEMA}
                value={text}
                height="clamp(320px, 52vh, 560px)"
                placeholder={EXAMPLE}
                autoFocus
                lineNumbers
                onChange={(v) => {
                  setText(v)
                  if (error) setError(null)
                }}
                onSubmit={submit}
              />
            </FormRow>
          </section>
        </div>
      </div>
      <McpFooter onCancel={onCancel} onSave={submit} saveDisabled={!text.trim()} saveLabel="Import" />
    </div>
  )
}
