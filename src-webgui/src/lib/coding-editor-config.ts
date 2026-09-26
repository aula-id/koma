import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import { getCodingConfig, editorPreferences } from './coding-config'
import { workspaceKey, type WorkspaceRef } from './coding-service'
import { useKoma } from '../store/koma'
import { languageIdForPath } from './monaco-lsp'

function binding(key: string): number | null {
  const parts = key.split('+')
  let code = 0
  for (const part of parts.slice(0, -1)) {
    if (part === 'Mod') code |= monaco.KeyMod.CtrlCmd
    else if (part === 'Shift') code |= monaco.KeyMod.Shift
    else if (part === 'Alt') code |= monaco.KeyMod.Alt
    else return null
  }
  const last = parts[parts.length - 1]
  const name = /^[a-z]$/i.test(last) ? `Key${last.toUpperCase()}` : /^\d$/.test(last) ? `Digit${last}` : last
  const value = monaco.KeyCode[name as keyof typeof monaco.KeyCode]
  return typeof value === 'number' ? code | value : null
}
export function configureCodingEditor(editor: monaco.editor.IStandaloneCodeEditor, workspace: WorkspaceRef, path: string) {
  let stopped = false
  let sequence = 0
  let bindings: monaco.IDisposable[] = []
  const report = (error: unknown) => useKoma.setState(s => {
    const text = `Coding settings: ${error instanceof Error ? error.message : String(error)}`
    if (s.ui.toast?.text === text) return s
    const id = s.ui.toastSeq + 1
    return { ui: { ...s.ui, toastSeq: id, toast: { id, kind: 'error', text } } }
  })
  const reload = async () => {
    const request = ++sequence
    let next: monaco.IDisposable[] = []
    try {
      const config = await getCodingConfig(workspace)
      if (stopped || request !== sequence) return
      const settings = editorPreferences(config, languageIdForPath(path))
      const tabSize = settings.tabSize == null ? 4 : settings.tabSize
      if (!Number.isInteger(tabSize) || (tabSize as number) < 1 || (tabSize as number) > 16) throw new Error('editor.tabSize must be an integer from 1 to 16')
      if (settings.insertSpaces != null && typeof settings.insertSpaces !== 'boolean') throw new Error('editor.insertSpaces must be a boolean')
      const wordWrap = settings.wordWrap ?? 'on'
      if (!['on', 'off', 'wordWrapColumn', 'bounded'].includes(String(wordWrap))) throw new Error('Invalid editor.wordWrap')
      editor.updateOptions({
        wordWrap: wordWrap as 'on' | 'off' | 'wordWrapColumn' | 'bounded',
        minimap: { enabled: settings.minimap === true },
        inlayHints: { enabled: settings.inlayHints === false ? 'off' : 'on' },
        lineNumbers: settings.lineNumbers === 'relative' ? 'relative' : settings.lineNumbers === 'off' ? 'off' : 'on',
        renderWhitespace: settings.renderWhitespace === 'all' ? 'all' : settings.renderWhitespace === 'none' ? 'none' : 'selection',
      })
      editor.getModel()?.updateOptions({ tabSize: tabSize as number, insertSpaces: settings.insertSpaces !== false })
      for (const [index, value] of (config.keybindings ?? []).entries()) {
        if (!value || typeof value !== 'object') continue
        const { key, command } = value as { key?: unknown; command?: unknown }
        if (typeof key !== 'string' || typeof command !== 'string') throw new Error('Each keybinding needs key and command strings')
        const keys = binding(key)
        if (keys == null) throw new Error(`Unsupported keybinding: ${key}`)
        if (command !== 'save' && !editor.getAction(command)) throw new Error(`Unknown editor command: ${command}`)
        next.push(editor.addAction({ id: `koma.project-key.${workspaceKey(workspace)}.${path}.${request}.${index}`, label: command, keybindings: [keys], run: () => {
          if (command === 'save') useKoma.getState().saveCodingFile(workspace.root, path)
          else return editor.getAction(command)?.run()
        } }))
      }
      for (const item of bindings) item.dispose()
      bindings = next
      next = []
    } catch (error) {
      for (const item of next) item.dispose()
      if (!stopped && request === sequence) report(error)
    }
  }
  const event = (event: Event) => { if (workspaceKey((event as CustomEvent<WorkspaceRef>).detail) === workspaceKey(workspace)) void reload() }
  const model = editor.onDidChangeModel(() => { void reload() })
  const focus = () => { void reload() }
  window.addEventListener('koma-coding-config', event)
  window.addEventListener('focus', focus)
  void reload()
  return () => {
    stopped = true
    model.dispose()
    for (const item of bindings) item.dispose()
    window.removeEventListener('koma-coding-config', event)
    window.removeEventListener('focus', focus)
  }
}
