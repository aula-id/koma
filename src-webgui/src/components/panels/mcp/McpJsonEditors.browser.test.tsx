import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'
import { render } from 'vitest-browser-react'
import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import '../../../styles.css'
import { McpEditView } from './McpEditView'
import { McpImportView } from './McpImportView'
import { HEADER_KEEP, blankDraft, draftToRequest, type McpDraft } from './mcpForm'

const until = async (fn: () => boolean, ms = 8000) => {
  const end = Date.now() + ms
  while (!fn()) {
    if (Date.now() > end) throw new Error('timed out')
    await new Promise((r) => setTimeout(r, 25))
  }
}
const ready = () => until(() => !!document.querySelector('[data-editor-state="ready"]'))
const model = () => monaco.editor.getModels().find((m) => m.uri.scheme === 'koma-embedded')!
const setText = (t: string) => model().setValue(t)
const button = (label: string) => [...document.querySelectorAll('button')].find((b) => b.textContent === label)!

function Edit({ initial, onSave }: { initial: McpDraft; onSave: (d?: McpDraft) => void }) {
  const [draft, setDraft] = useState(initial)
  return (
    <div style={{ width: 900, height: 700, display: 'flex', flexDirection: 'column' }} className="bg-koma-bg">
      <McpEditView draft={draft} isNew={false} onChange={(p) => setDraft((d) => ({ ...d, ...p }))} onSave={onSave} onCancel={vi.fn()} />
    </div>
  )
}

describe('MCP import editor', () => {
  it('starts as a textarea, upgrades to Monaco, and shows the parser error without losing the text', async () => {
    const onImport = vi.fn()
    await render(
      <div style={{ width: 900, height: 700, display: 'flex', flexDirection: 'column' }} className="bg-koma-bg">
        <McpImportView onImport={onImport} onCancel={vi.fn()} />
      </div>,
    )
    expect(document.querySelector('[data-testid="mcp-import-editor"]')).not.toBeNull()
    await ready()
    expect(document.querySelectorAll('.monaco-editor')).toHaveLength(1)
    expect(model().getLanguageId()).toBe('json')

    setText('{ nope')
    await until(() => button('Import').disabled === false)
    await userEvent.click(button('Import'))
    await until(() => (document.body.textContent ?? '').includes('not valid JSON'))
    expect(onImport).not.toHaveBeenCalled()
    expect(model().getValue()).toBe('{ nope')

    setText('{"mcpServers":{"a":{"command":"npx","args":["-y","x"]}}}')
    await until(() => !(document.body.textContent ?? '').includes('not valid JSON'))
    await userEvent.click(button('Import'))
    expect(onImport).toHaveBeenCalledWith('{"mcpServers":{"a":{"command":"npx","args":["-y","x"]}}}')
  })

  it('Ctrl/Cmd+Enter imports from inside the editor, and unmount disposes the model', async () => {
    const onImport = vi.fn()
    const view = await render(<McpImportView onImport={onImport} onCancel={vi.fn()} />)
    await ready()
    setText('{"a":{"command":"c"}}')
    ;(document.querySelector('.monaco-editor textarea') as HTMLElement).focus()
    const mod = /Mac|iPhone|iPad/.test(navigator.platform) ? 'Meta' : 'Control'
    await userEvent.keyboard(`{${mod}>}{Enter}{/${mod}}`)
    await until(() => onImport.mock.calls.length > 0)
    view.unmount()
    expect(monaco.editor.getModels().filter((m) => m.uri.scheme === 'koma-embedded')).toHaveLength(0)
  })

  it('the schema accepts the three shapes and flags a wrong field type', async () => {
    await render(<McpImportView onImport={vi.fn()} onCancel={vi.fn()} />)
    await ready()
    const markers = () => monaco.editor.getModelMarkers({ resource: model().uri })
    for (const ok of [
      '{"mcpServers":{"a":{"command":"npx","args":["-y"],"env":{"K":"v"},"cwd":"/x"}}}',
      '{"servers":{"b":{"type":"http","url":"https://h","headers":{"A":"b"}}}}',
      '{"c":{"type":"sse","url":"https://h"},"d":{"command":"x"}}',
    ]) {
      setText(ok)
      await new Promise((r) => setTimeout(r, 600))
      expect(markers().map((m) => m.message), ok).toEqual([])
    }
    setText('{"mcpServers":{"a":{"command":5}}}')
    await until(() => markers().length > 0)
  })
})

describe('MCP edit JSON view', () => {
  const base: McpDraft = {
    ...blankDraft(), id: 'u2', name: 'remote', transport: 'http', url: 'https://x.test/mcp',
    headersText: `Authorization: ${HEADER_KEEP}`,
  }

  it('shows the server, keeps masked headers, blocks invalid JSON, and saves through the shared draft', async () => {
    const onSave = vi.fn()
    await render(<Edit initial={base} onSave={onSave} />)
    await userEvent.click(button('JSON'))
    await ready()
    expect(JSON.parse(model().getValue())).toEqual({ remote: { type: 'http', url: 'https://x.test/mcp', headers: { Authorization: HEADER_KEEP } } })

    setText('{ "remote": ')
    await userEvent.click(button('Form'))
    await until(() => (document.body.textContent ?? '').includes('not valid JSON'))
    expect(model().getValue()).toBe('{ "remote": ')
    expect(document.querySelector('section[aria-label="Essentials"]')).toBeNull()
    await userEvent.click(button('Save'))
    expect(onSave).not.toHaveBeenCalled()

    setText('{ "remote": { "url": "https://y.test/mcp", "headers": { "Authorization": "' + HEADER_KEEP + '" }, "toolTimeoutMs": 2000 } }')
    await userEvent.click(button('Save'))
    expect(onSave).toHaveBeenCalledTimes(1)
    const saved = onSave.mock.calls[0][0] as McpDraft
    const req = draftToRequest(saved, false)
    expect(req.url).toBe('https://y.test/mcp')
    expect(req.extras.headers).toEqual([['Authorization', HEADER_KEEP]])
    expect(req.extras.toolTimeoutMs).toBe(2000)
  })

  it('switching back to Form applies the JSON edits', async () => {
    await render(<Edit initial={base} onSave={vi.fn()} />)
    await userEvent.click(button('JSON'))
    await ready()
    setText('{ "renamed": { "type": "http", "url": "https://z.test", "disabled": true } }')
    await userEvent.click(button('Form'))
    await until(() => !!document.querySelector('section[aria-label="Essentials"]'))
    expect((document.querySelector('input[placeholder="e.g. filesystem"]') as HTMLInputElement).value).toBe('renamed')
    expect((document.querySelector('input[placeholder="https://…"]') as HTMLInputElement).value).toBe('https://z.test')
  })
})
