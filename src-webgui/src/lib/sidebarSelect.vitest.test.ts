import { expect, it } from 'vitest'
import { resolveViewSelection } from './sidebarSelect'

it('chat icon on the focused chat tab collapses an open chat view', () => {
  expect(resolveViewSelection('explore', 'explore', true, 'chat')).toEqual({ activateChat: true, view: 'explore', sidebarOpen: false })
})
it('chat icon with another tab focused focuses chat and keeps the sidebar open', () => {
  expect(resolveViewSelection('explore', 'explore', true, 'settings')).toEqual({ activateChat: true, view: 'explore', sidebarOpen: true })
})
it('chat icon reopens a collapsed chat view', () => {
  expect(resolveViewSelection('explore', 'explore', false, 'chat').sidebarOpen).toBe(true)
})
it('chat icon from another view switches, opens and focuses chat', () => {
  for (const open of [true, false]) {
    for (const tab of ['chat', 'settings']) {
      expect(resolveViewSelection('explore', 'git', open, tab)).toEqual({ activateChat: true, view: 'explore', sidebarOpen: true })
    }
  }
})
it('other views keep the plain toggle and switch', () => {
  expect(resolveViewSelection('git', 'git', true, 'chat')).toEqual({ activateChat: false, view: 'git', sidebarOpen: false })
  expect(resolveViewSelection('git', 'git', false, 'chat').sidebarOpen).toBe(true)
  expect(resolveViewSelection('git', 'mcp', false, 'settings')).toEqual({ activateChat: false, view: 'git', sidebarOpen: true })
})
