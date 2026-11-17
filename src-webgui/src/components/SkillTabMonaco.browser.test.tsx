import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'
import { render } from 'vitest-browser-react'
import * as monaco from 'monaco-editor/esm/vs/editor/editor.api'
import '../styles.css'
import { useKoma, type SkillCatalogueEntry, type SkillDetail } from '../store/koma'
import SkillTab from './SkillTab'

const entry: SkillCatalogueEntry = {
  skillId: 'opaque-a', generation: 'gen-a', name: 'alpha', description: 'Draft description', triggers: '',
  sourceTier: 'global', scope: 'global', sourcePath: '/skills/alpha/SKILL.md',
}
const detail: SkillDetail = {
  skillId: 'opaque-a', generation: 'gen-a', name: 'alpha', description: 'Draft description', triggers: '',
  declaredTools: ['Read'], instruction: '---\nname: alpha\n---\n# Draft body', companionFiles: ['scripts/run.py'], scope: 'global',
  sourcePath: '/skills/alpha/SKILL.md', editable: true, structuredSaveSupported: true,
}
const tab = { id: 'skill:opaque-a', kind: 'skill' as const, skillId: 'opaque-a', title: 'alpha' }

const until = async (fn: () => boolean, ms = 8000) => {
  const end = Date.now() + ms
  while (!fn()) {
    if (Date.now() > end) throw new Error('timed out')
    await new Promise((r) => setTimeout(r, 25))
  }
}
const ready = () => until(() => !!document.querySelector('[data-editor-state="ready"]'))
const model = () => monaco.editor.getModels().find((m) => m.uri.scheme === 'koma-embedded')!
const requests = () => vi.mocked(window.ipc!.postMessage).mock.calls.map((c) => JSON.parse(c[0] as string))
const setup = (d: SkillDetail, e: SkillCatalogueEntry) => {
  window.ipc = { postMessage: vi.fn() }
  useKoma.setState({ skills: [e], loadedSkillNames: [], skillDetails: { [tab.id]: d }, skillDetailPending: {}, skillDetailErrors: {}, skillFiles: {}, skillOpResults: {}, skillSessionEpoch: 4 })
}
const mount = () => render(<div style={{ width: 900, height: 900, display: 'flex', flexDirection: 'column' }} className="bg-koma-bg"><SkillTab tab={tab} /></div>)

afterEach(() => { delete window.ipc })

describe('SkillTab Monaco editors', () => {
  beforeEach(() => setup(detail, entry))

  it('edits SKILL.md as markdown, keeps front-matter, and Save sends the edited text', async () => {
    await mount()
    await ready()
    expect(model().getLanguageId()).toBe('markdown')
    expect(model().getValue()).toBe(detail.instruction)
    model().setValue(`${detail.instruction}\nmore`)
    const save = [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Save')!
    await userEvent.click(save)
    const sent = requests().find((r) => r.r === 'UpdateSkill')
    expect(sent.instruction).toBe(`${detail.instruction}\nmore`)
    expect(sent.name).toBe('alpha')
    expect(sent.description).toBe('Draft description')
  })

  it('Ctrl/Cmd+S in the editor saves once', async () => {
    await mount()
    await ready()
    model().setValue('# changed')
    await userEvent.click(document.querySelector<HTMLElement>('[data-testid="skill-instruction-editor"] .view-lines')!)
    await until(() => !!document.activeElement?.closest('[data-testid="skill-instruction-editor"]'))
    const mod = /Mac|iPhone|iPad/.test(navigator.platform) ? 'Meta' : 'Control'
    await userEvent.keyboard(`{${mod}>}s{/${mod}}`)
    await until(() => requests().some((r) => r.r === 'UpdateSkill'))
    expect(requests().filter((r) => r.r === 'UpdateSkill')).toHaveLength(1)
    expect(requests().find((r) => r.r === 'UpdateSkill').instruction).toBe('# changed')
  })

  it('shows a companion file read-only with the language from its extension', async () => {
    await mount()
    await userEvent.click([...document.querySelectorAll('[role="tab"]')].find((t) => t.textContent === 'scripts/run.py')!)
    useKoma.setState((s) => ({ skillFiles: { ...s.skillFiles, [`${tab.id}:scripts/run.py`]: { content: 'print(1)\n' } as never } }))
    await until(() => !!document.querySelector('[data-testid="skill-companion-editor"][data-editor-state="ready"]'))
    expect(model().getLanguageId()).toBe('python')
    expect(model().getValue()).toBe('print(1)\n')
    expect(document.querySelector('[data-testid="skill-companion-editor"] textarea')?.hasAttribute('readonly')).toBe(true)
  })

  it('an external skill editor is read-only and cannot be saved', async () => {
    setup({ ...detail, scope: 'external', editable: false }, { ...entry, sourceTier: 'external', scope: 'external' } as SkillCatalogueEntry)
    await mount()
    await ready()
    const input = document.querySelector('[data-testid="skill-instruction-editor"] textarea')!
    expect(input.hasAttribute('readonly')).toBe(true)
    expect(requests().some((r) => r.r === 'UpdateSkill')).toBe(false)
  })
})
