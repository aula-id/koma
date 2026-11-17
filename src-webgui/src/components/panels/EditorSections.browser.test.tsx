import { beforeEach, describe, expect, it } from 'vitest'
import { render } from 'vitest-browser-react'
import { useKoma, type Tab } from '../../store/koma'
import { DiagramPanel } from './DiagramPanel'
import { DesignPanel } from './DesignPanel'

const diagram = { id: 'diagram-1', kind: 'diagram', root: '/r', path: 'a.diagram' } as unknown as Tab
const design = { id: 'design-1', kind: 'design', root: '/r', path: 'a.design' } as unknown as Tab
const chat = { id: 'chat', kind: 'chat' } as unknown as Tab

function setTabs(tabs: Tab[], active: string) {
  const ui = useKoma.getState().ui
  useKoma.setState({
    req: () => undefined,
    ui: { ...ui, tabs, activeTabId: active, groupActive: { ...(ui.groupActive ?? {}), [Object.keys(ui.groupActive ?? { main: 1 })[0]]: active } },
  } as never)
}

const has = (container: HTMLElement, title: string) => [...container.querySelectorAll('*')].some((el) => el.children.length === 0 && el.textContent?.trim().toLowerCase() === title)

describe('Shapes and Layers sections follow the active tab', () => {
  beforeEach(() => {
    const st = useKoma.getState()
    useKoma.setState({
      session: { ...st.session, id: 's1' },
      settingsValues: { ...(st.settingsValues ?? {}), workdir: ['/r'] },
      coding: { ...st.coding, activeRoot: '/r' },
    } as never)
    setTabs([chat], 'chat')
  })

  it('Shapes: hidden with no diagram tab, hidden when inactive, shown when active', async () => {
    const view = await render(<DiagramPanel />)
    expect(has(view.container, 'shapes')).toBe(false)
    setTabs([chat, diagram], 'chat')
    await expect.poll(() => has(view.container, 'shapes')).toBe(false)
    setTabs([chat, diagram], 'diagram-1')
    await expect.poll(() => has(view.container, 'shapes')).toBe(true)
    expect(has(view.container, 'diagrams')).toBe(true)
  })

  it('Layers: hidden with no design tab, hidden when inactive, shown when active', async () => {
    const view = await render(<DesignPanel />)
    expect(has(view.container, 'layers')).toBe(false)
    setTabs([chat, design], 'chat')
    await expect.poll(() => has(view.container, 'layers')).toBe(false)
    setTabs([chat, design], 'design-1')
    await expect.poll(() => has(view.container, 'layers')).toBe(true)
  })
})
