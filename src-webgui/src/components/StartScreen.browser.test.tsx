import '../styles.css'
import { expect, it, vi } from 'vitest'
import { render } from 'vitest-browser-react'
import { useKoma } from '../store/koma'
import { StartScreen } from './StartScreen'

it('starts a fresh session in the selected recent folder from the detached screen', async () => {
  const req = vi.fn()
  const folder = '/projects/client two/docs'
  useKoma.setState((state) => ({
    req,
    hub: {
      ...state.hub,
      state: 'swapper',
      cooking: [],
      history: [
        { id: 'first', name: 'First', lastActive: 1, dirLabel: 'docs', workdir: '/projects/client one/docs', currentDir: false },
        { id: 'second', name: 'Second', lastActive: 2, dirLabel: 'docs', workdir: folder, currentDir: false },
      ],
    },
  }))

  const screen = await render(<StartScreen />)
  expect(req.mock.calls.filter(([request]) => request.r !== 'RefreshHub')).toEqual([])
  await screen.getByRole('button', { name: `New session in ${folder}`, exact: true }).click()
  expect(req.mock.calls.filter(([request]) => request.r !== 'RefreshHub')).toEqual([
    [{ r: 'NewSession', workdir: folder }],
  ])
})
