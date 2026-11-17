import { describe, expect, it } from 'vitest'
import { render } from '@testing-library/react'
import { PanelActions, PanelActionsSlot, baseName } from './PanelChrome'

describe('baseName', () => {
  it.each([
    ['C:\\a\\b\\proj', 'proj'],
    ['C:/a/b/', 'b'],
    ['//host/share', 'share'],
    ['/', '/'],
    ['', ''],
    ['proj', 'proj'],
  ])('%s -> %s', (input, expected) => expect(baseName(input)).toBe(expected))
})

describe('PanelActions', () => {
  it('renders inline without a provider', () => {
    const { container } = render(<PanelActions><b>x</b></PanelActions>)
    expect(container.querySelector('b')).not.toBeNull()
  })
  it('renders nothing while the slot is unmounted', () => {
    const { container } = render(<PanelActionsSlot.Provider value={null}><PanelActions><b>x</b></PanelActions></PanelActionsSlot.Provider>)
    expect(container.querySelector('b')).toBeNull()
  })
  it('portals into the slot element', () => {
    const slot = document.createElement('div')
    const { container } = render(<PanelActionsSlot.Provider value={slot}><PanelActions><b>x</b></PanelActions></PanelActionsSlot.Provider>)
    expect(container.querySelector('b')).toBeNull()
    expect(slot.querySelector('b')).not.toBeNull()
  })
})
