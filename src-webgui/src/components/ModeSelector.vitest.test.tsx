import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useKoma } from '../store/koma'
import { ModeSelector } from './ModeSelector'

beforeEach(() => {
  window.ipc = { postMessage: vi.fn() }
  useKoma.setState((state) => ({
    session: { ...state.session, mode: 'sdlc' },
  }))
})

afterEach(() => {
  cleanup()
})

it('shows a live SDLC session as SDLC and leaves SDLC out of the menu', () => {
  render(<ModeSelector />)
  fireEvent.click(screen.getByRole('button', { name: 'SDLC' }))

  const labels = screen.getAllByRole('button').map((button) => button.textContent?.trim())
  expect(labels.filter((label) => label === 'SDLC')).toEqual(['SDLC'])
  expect(labels).toEqual(expect.arrayContaining(['SDLC', 'Auto', 'Plan', 'Normal']))
  expect(screen.getByRole('button', { name: 'Auto' }).className).toContain('opacity-75')

  fireEvent.mouseDown(screen.getByRole('button', { name: 'Auto' }))
  expect(window.ipc.postMessage).toHaveBeenCalledWith(
    JSON.stringify({ t: 'req', r: 'SetMode', mode: 'auto' }),
  )
})
