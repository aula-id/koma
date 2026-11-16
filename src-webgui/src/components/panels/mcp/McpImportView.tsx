import { useState } from 'react'
import { Field, TextArea } from '../form'
import { DetailHeader, FormActions } from '../helpers'

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

export function McpImportView({ onImport, onCancel }: Props) {
  const [text, setText] = useState('')
  return (
    <>
      <DetailHeader onBack={onCancel} title="Import JSON" />
      <div className="flex-1 overflow-auto py-1">
        <Field label="MCP servers JSON">
          <TextArea
            value={text}
            rows={12}
            autoFocus
            spellCheck={false}
            placeholder={EXAMPLE}
            onChange={(e) => setText(e.target.value)}
          />
          <span className="text-[10px] text-koma-fg opacity-40">
            Paste the mcpServers snippet from a server&apos;s README (Claude Code, Cursor or VS Code
            format).
          </span>
        </Field>
      </div>
      <FormActions
        onCancel={onCancel}
        onSave={() => onImport(text)}
        saveDisabled={!text.trim()}
        saveLabel="Import"
      />
    </>
  )
}
