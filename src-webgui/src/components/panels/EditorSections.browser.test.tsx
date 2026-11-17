import { beforeEach, describe, expect, it } from 'vitest'
import { render } from 'vitest-browser-react'
import { useKoma, type Tab } from '../../store/koma'
import { DiagramPanel } from './DiagramPanel'
import { DesignPanel } from './DesignPanel'

const diagram = { id: 'diagram-1', kind: 'diagram', root: '/r', path: 'a.diagram' } as unknown as Tab
const design = { id: 'design-1', kind: 'design', root: '/r', path: 'a.design' } as unknown as Tab
const welcomeDiagram = { id: 'welcome-diagram', kind: 'welcome', which: 'diagram' } as unknown as Tab
const welcomeDesign = { id: 'welcome-design', kind: 'welcome', which: 'design' } as unknown as Tab
const chat = { id: 'chat', kind: 'chat' } as unknown as Tab

function setTabs(tabs: Tab[], active: string) {
  const ui = useKoma.getState().ui
  useKoma.setState({
    req: () => undefined,
    ui: { ...ui, tabs, activeTabId: active, groupActive: { ...(ui.groupActive ?? {}), [Object.keys(ui.groupActive ?? { main: 1 })[0]]: active } },
  } as never)
}

const has = (container: HTMLElement, title: string) => [...container.querySelectorAll('*')].some((el) => el.children.length === 0 && el.textContent?.trim().toLowerCase() === title)

describe('Diagram and Design panel sections follow the active tab', () => {
  beforeEach(() => {
    const st = useKoma.getState()
    useKoma.setState({
      session: { ...st.session, id: 's1' },
      settingsValues: { ...(st.settingsValues ?? {}), workdir: ['/r'] },
      coding: { ...st.coding, activeRoot: '/r' },
    } as never)
    setTabs([chat], 'chat')
  })

  it('Diagram: editor active shows Shapes and hides the file list; otherwise the reverse', async () => {
    const view = await render(<DiagramPanel />)
    expect(has(view.container, 'shapes')).toBe(false)
    expect(has(view.container, 'diagrams')).toBe(true)
    setTabs([chat, diagram], 'chat')
    await expect.poll(() => has(view.container, 'shapes')).toBe(false)
    setTabs([chat, diagram, welcomeDiagram], 'welcome-diagram')
    await expect.poll(() => has(view.container, 'diagrams')).toBe(true)
    expect(has(view.container, 'shapes')).toBe(false)
    setTabs([chat, diagram], 'diagram-1')
    await expect.poll(() => has(view.container, 'shapes')).toBe(true)
    expect(has(view.container, 'diagrams')).toBe(false)
  })

  it('Diagram: the Files toggle brings the list back while editing', async () => {
    setTabs([chat, diagram], 'diagram-1')
    const view = await render(<DiagramPanel />)
    await expect.poll(() => has(view.container, 'shapes')).toBe(true)
    ;(view.container.querySelector('[aria-label="Files"]') as HTMLElement).click()
    await expect.poll(() => has(view.container, 'diagrams')).toBe(true)
    expect(has(view.container, 'shapes')).toBe(false)
  })

  it('Design: editor active shows Layers and hides the file list; otherwise the reverse', async () => {
    const view = await render(<DesignPanel />)
    expect(has(view.container, 'layers')).toBe(false)
    expect(has(view.container, 'files')).toBe(true)
    setTabs([chat, design], 'chat')
    await expect.poll(() => has(view.container, 'layers')).toBe(false)
    setTabs([chat, design, welcomeDesign], 'welcome-design')
    await expect.poll(() => has(view.container, 'files')).toBe(true)
    expect(has(view.container, 'layers')).toBe(false)
    setTabs([chat, design], 'design-1')
    await expect.poll(() => has(view.container, 'layers')).toBe(true)
    expect(has(view.container, 'files')).toBe(false)
  })

  it('Design: the Files toggle brings the list back while editing', async () => {
    setTabs([chat, design], 'design-1')
    const view = await render(<DesignPanel />)
    await expect.poll(() => has(view.container, 'layers')).toBe(true)
    ;(view.container.querySelector('[aria-label="Files"]') as HTMLElement).click()
    await expect.poll(() => has(view.container, 'files')).toBe(true)
    expect(has(view.container, 'layers')).toBe(false)
  })
})
