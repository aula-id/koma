import { useState } from 'react'
import { TextArea } from '../form'
import { FormRow } from './McpGroup'
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
      <div className="min-h-0 min-w-0 flex-1 overflow-y-auto overflow-x-hidden px-3 py-4">
        <FormRow
          label="MCP servers JSON"
          htmlFor="mcp-import-json"
          hint="Paste the mcpServers snippet from a server's README (Claude Code, Cursor or VS Code format)."
        >
          <TextArea
            id="mcp-import-json"
            value={text}
            rows={12}
            autoFocus
            spellCheck={false}
            placeholder={EXAMPLE}
            onChange={(e) => setText(e.target.value)}
          />
        </FormRow>
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
