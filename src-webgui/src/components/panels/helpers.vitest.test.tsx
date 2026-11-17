// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { Plug } from 'lucide-react'
import { useKoma } from '../../store/koma'
import { AddBtn, Empty, EmptyState, NoWorkspaceState, Row } from './helpers'

afterEach(cleanup)

describe('EmptyState', () => {
  it('renders title, body and fires both actions', () => {
    const p = vi.fn()
    const s = vi.fn()
    render(
      <EmptyState icon={Plug} title="Connect an AI" primary={{ label: 'Sign in', onClick: p }} secondary={{ label: 'Add key', onClick: s }}>
        Body text
      </EmptyState>,
    )
    expect(screen.getByRole('region', { name: 'Connect an AI' })).toBeTruthy()
    expect(screen.getByText('Body text')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Sign in' }))
    fireEvent.click(screen.getByRole('button', { name: 'Add key' }))
    expect(p).toHaveBeenCalledTimes(1)
    expect(s).toHaveBeenCalledTimes(1)
  })

  it('links a disabled primary to its visible reason', () => {
    render(
      <EmptyState title="T" primary={{ label: 'Go', onClick: vi.fn(), disabled: true, disabledReason: 'Open a project first' }}>
        b
      </EmptyState>,
    )
    const btn = screen.getByRole('button', { name: 'Go' }) as HTMLButtonElement
    expect(btn.disabled).toBe(true)
    const reason = screen.getByText('Open a project first')
    expect(btn.getAttribute('aria-describedby')).toBe(reason.id)
  })
})

describe('AddBtn', () => {
  it('keeps the full accessible name with a visible text', () => {
    const { container } = render(<AddBtn label="Add provider" text="Add" tourId="t-id" onClick={vi.fn()} />)
    const btn = screen.getByRole('button', { name: 'Add provider' })
    expect(btn.getAttribute('title')).toBe('Add provider')
    expect(btn.getAttribute('data-tour')).toBe('t-id')
    expect(container.querySelector('span')?.textContent).toBe('Add')
  })
  it('has no text span without text', () => {
    const { container } = render(<AddBtn label="Add model" onClick={vi.fn()} />)
    expect(container.querySelector('span')).toBeNull()
  })
})

describe('Empty', () => {
  it('renders children', () => {
    render(<Empty>Nothing here</Empty>)
    expect(screen.getByText('Nothing here')).toBeTruthy()
  })
})

describe('NoWorkspaceState', () => {
  it('opens settings', () => {
    const prev = useKoma.getState().openSettingsTab
    const fn = vi.fn()
    useKoma.setState({ openSettingsTab: fn })
    render(<NoWorkspaceState />)
    fireEvent.click(screen.getByRole('button', { name: 'Open Settings' }))
    expect(fn).toHaveBeenCalledTimes(1)
    useKoma.setState({ openSettingsTab: prev })
  })
})

describe('Row', () => {
  it('reveals its action gutter for keyboard focus', () => {
    const { container } = render(
      <Row title="x" confirmLabel="c" armed={false} onEdit={vi.fn()} onArm={vi.fn()} onDisarm={vi.fn()} onConfirm={vi.fn()} />,
    )
    const gutter = container.querySelector('.max-w-0') as HTMLElement
    expect(gutter.className).toContain('group-has-[:focus-visible]:opacity-100')
    expect(gutter.className).toContain('group-has-[:focus-visible]:max-w-[44px]')
  })
})
