import { beforeEach, describe, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'
import { render } from 'vitest-browser-react'
import WelcomeTab from './WelcomeTab'
import { useKoma } from '../store/koma'
import { fileKey } from '../store/coding'

const req = vi.fn()
const createDiagramFile = vi.fn()
const openDiagramTab = vi.fn()

async function button(text: string) {
  const target = [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === text)
  if (!target) throw new Error(`Missing button ${text}`)
  return target
}

beforeEach(() => {
  vi.clearAllMocks()
  useKoma.setState((s) => ({
    req,
    createDiagramFile,
    openDiagramTab,
    session: { ...s.session, id: 'browser-session' },
    settingsValues: { ...s.settingsValues, workdir: ['/w'] } as typeof s.settingsValues,
    diagram: { ...s.diagram, pendingCreate: null },
    coding: {
      ...s.coding,
      activeRoot: '/w',
      dirs: { [fileKey('/w', '.koma')]: { entries: [], loading: false, error: null } },
    },
  }))
})

describe('WelcomeTab', () => {
  it('creates a diagram from the inline name input', async () => {
    await render(<WelcomeTab which="diagram" />)
    expect(document.querySelector('h1')?.textContent).toBe('Diagrams')
    await userEvent.click(await button('New diagram'))
    await userEvent.type(document.querySelector<HTMLInputElement>('input[placeholder="name.diag"]')!, 'flow{Enter}')
    expect(createDiagramFile).toHaveBeenCalledWith('/w', '.koma/flow.diag')
  })

  it('disables New while a create is pending', async () => {
    useKoma.setState((s) => ({ diagram: { ...s.diagram, pendingCreate: { path: '.koma/x.diag' } as never } }))
    await render(<WelcomeTab which="diagram" />)
    expect((await button('New diagram') as HTMLButtonElement).disabled).toBe(true)
  })

  it('lists existing files and opens one', async () => {
    useKoma.setState((s) => ({
      coding: { ...s.coding, dirs: { [fileKey('/w', '.koma')]: { entries: [{ name: 'a.diag', path: '.koma/a.diag', isDir: false }], loading: false, error: null } } },
    }))
    await render(<WelcomeTab which="diagram" />)
    await userEvent.click(await button('a.diag'))
    expect(openDiagramTab).toHaveBeenCalledWith('/w', '.koma/a.diag')
  })

  it('offers .fig import for design only', async () => {
    await render(<WelcomeTab which="design" />)
    await button('New design')
    await button('Import .fig')
  })

  it('asks for a project when no session is open', async () => {
    useKoma.setState((s) => ({ session: { ...s.session, id: null } }))
    await render(<WelcomeTab which="design" />)
    expect(document.body.textContent).toContain('Open a project to use Design')
  })
})
