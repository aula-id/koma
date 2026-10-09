import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { canvasPoint, isUiScale, readUiScale, restoreUiScale, setUiScale, UI_SCALE_KEY, useUiScale } from './uiScale'

beforeEach(() => {
  localStorage.clear()
  useUiScale.setState({ scale: 1, pending: false, error: null })
  delete window.ipc
  window.location.hash = ''
})
afterEach(() => { vi.restoreAllMocks(); delete window.ipc })

describe('installation UI scale', () => {
  it('accepts exactly the advertised numeric values', () => {
    for (const scale of [1, 1.5, 2, 2.5]) expect(isUiScale(scale)).toBe(true)
    for (const scale of [0, -1, 1.25, 3, NaN, Infinity, '2', null]) expect(isUiScale(scale)).toBe(false)
  })
  it('defaults to original sizes and rejects corrupt or unsupported preferences', () => {
    expect(readUiScale()).toBe(1)
    for (const raw of ['oops', '3', 'null', '"2"', '{}']) {
      localStorage.setItem(UI_SCALE_KEY, raw)
      expect(readUiScale()).toBe(1)
    }
  })
  it('persists and restores without compounding', async () => {
    await setUiScale(2.5)
    expect(localStorage.getItem(UI_SCALE_KEY)).toBe('2.5')
    useUiScale.setState({ scale: 1 })
    await restoreUiScale()
    await restoreUiScale()
    expect(useUiScale.getState().scale).toBe(2.5)
    expect(document.documentElement.style.zoom).toBe('2.5')
    await setUiScale(1)
    expect(document.documentElement.style.zoom).toBe('1')
  })
  it('stays usable when storage is unavailable', async () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked') })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked') })
    expect(readUiScale()).toBe(1)
    expect(await setUiScale(2)).toBe(true)
    expect(useUiScale.getState().scale).toBe(2)
  })
  it('awaits native success and ignores stale acknowledgements', async () => {
    const postMessage = vi.fn()
    window.ipc = { postMessage }
    document.documentElement.style.zoom = ''
    const changing = setUiScale(1.5)
    const request = JSON.parse(postMessage.mock.calls[0][0])
    expect(request).toMatchObject({ t: 'req', r: 'SetUiScale', scale: 1.5 })
    window.__komaUiScaleReply?.({ requestId: 'stale', scale: 1.5, error: null })
    expect(useUiScale.getState().pending).toBe(true)
    expect(useUiScale.getState().scale).toBe(1)
    window.__komaUiScaleReply?.({ requestId: request.requestId, scale: 1.5, error: null })
    expect(await changing).toBe(true)
    expect(document.documentElement.style.zoom).toBe('')
    expect(readUiScale()).toBe(1.5)
  })
  it('retains the previous selection on native rejection or bridge failure', async () => {
    localStorage.setItem(UI_SCALE_KEY, '1.5')
    useUiScale.setState({ scale: 1.5 })
    window.ipc = { postMessage: raw => {
      const request = JSON.parse(raw)
      window.__komaUiScaleReply?.({ requestId: request.requestId, scale: request.scale, error: 'Zoom unavailable' })
    } }
    expect(await setUiScale(2)).toBe(false)
    expect(useUiScale.getState()).toMatchObject({ scale: 1.5, pending: false, error: expect.stringContaining('Zoom unavailable') })
    expect(readUiScale()).toBe(1.5)
    window.ipc.postMessage = () => { throw new Error('IPC unavailable') }
    expect(await setUiScale(2)).toBe(false)
    expect(useUiScale.getState().scale).toBe(1.5)
  })
  it('leaves the separate computer preview unscaled', async () => {
    localStorage.setItem(UI_SCALE_KEY, '2.5')
    window.location.hash = '#computer-preview'
    await restoreUiScale()
    expect(useUiScale.getState().scale).toBe(1)
  })
})

it('converts native pointer coordinates for canvases independently from GUI zoom', () => {
  window.ipc = { postMessage: vi.fn() }
  useUiScale.setState({ scale: 2.5 })
  expect(canvasPoint(80)).toBe(200)
  delete window.ipc
  expect(canvasPoint(200)).toBe(200)
})
