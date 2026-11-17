import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { ConnectorListView } from './ConnectorListView'

let stylesheet: HTMLLinkElement

beforeAll(async () => {
  const built = await fetch('/dist/index.html').then((response) => response.text())
  const cssHref = built.match(/href="\.\/(assets\/index-[^"]+\.css)"/)?.[1]
  if (!cssHref) throw new Error('production CSS asset not found; run vite build before browser tests')
  stylesheet = document.createElement('link')
  stylesheet.rel = 'stylesheet'
  stylesheet.href = `/dist/${cssHref}`
  document.head.append(stylesheet)
  await new Promise<void>((resolve, reject) => {
    stylesheet.addEventListener('load', () => resolve(), { once: true })
    stylesheet.addEventListener('error', () => reject(new Error('failed to load Koma CSS')), { once: true })
  })
})

afterAll(() => stylesheet.remove())

const handlers = () => ({
  onAddProvider: vi.fn(), onAddModel: vi.fn(), onConnectOAuth: vi.fn(),
  onEditProvider: vi.fn(), onEditModel: vi.fn(), onArm: vi.fn(), onDisarm: vi.fn(),
  onConfirmProvider: vi.fn(), onConfirmOAuth: vi.fn(), onConfirmModel: vi.fn(),
})

const provider = { id: 'p1', name: 'Acme', endpoint: 'https://api.example.test', hasKey: true }

async function mount(width: number, providers: typeof provider[] = [], h = handlers()) {
  await render(
    <div style={{ position: 'relative', width, height: 600 }}>
      <ConnectorListView providers={providers} conns={[]} models={[]} armed={null} {...h} />
    </div>,
  )
  return { h }
}

describe('ConnectorListView', () => {
  it('shows the Add word at 240px and hides it at 150px while keeping the full name', async () => {
    await mount(240)
    const wide = document.querySelector<HTMLElement>('[data-tour="connector-add-provider"] span')!
    expect(wide.getBoundingClientRect().width).toBeGreaterThan(0)
    document.body.innerHTML = ''
    await mount(150)
    const narrow = document.querySelector<HTMLElement>('[data-tour="connector-add-provider"] span')!
    expect(getComputedStyle(narrow).display).toBe('none')
    expect(document.querySelector('[data-tour="connector-add-provider"]')!.getAttribute('aria-label')).toBe('Add provider')
  })

  it('shows the welcome block when everything is empty and keeps one element per tour id', async () => {
    const { h } = await mount(240)
    expect(document.querySelector('[role="region"][aria-label="Connect an AI"]')).not.toBeNull()
    for (const id of ['connector-add-provider', 'connector-add-oauth', 'connector-add-model']) {
      expect(document.querySelectorAll(`[data-tour="${id}"]`).length).toBe(1)
    }
    const buttons = document.querySelectorAll<HTMLButtonElement>('[role="region"] button')
    buttons[0].click()
    expect(h.onConnectOAuth).toHaveBeenCalledTimes(1)
    buttons[1].click()
    expect(h.onAddProvider).toHaveBeenCalledTimes(1)
  })

  it('drops the welcome block and shows a count once a provider exists', async () => {
    await mount(240, [provider])
    expect(document.querySelector('[role="region"][aria-label="Connect an AI"]')).toBeNull()
    const header = document.querySelector('[data-tour="connector-add-provider"]')!.closest('.group')!
    expect(header.textContent).toContain('Providers1')
  })
})
