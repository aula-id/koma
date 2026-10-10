import { beforeEach, describe, expect, it } from 'vitest'
import { useComputerPreview } from './computerPreview'

beforeEach(() => {
  useComputerPreview.setState({ session: null, requestedSession: null, dismissedSession: null })
})

describe('computer preview visibility', () => {
  it('opens when sharing is already enabled, without a pending show request', () => {
    useComputerPreview.getState().syncController({ session: 's1', enabled: true })
    expect(useComputerPreview.getState().session).toBe('s1')
  })

  it('stays closed after an explicit hide until the user opens it or sharing stops', () => {
    const preview = useComputerPreview.getState()
    preview.syncController({ session: 's1', enabled: true })
    preview.dismiss('s1')
    preview.syncController({ session: 's1', enabled: true })
    expect(useComputerPreview.getState().session).toBeNull()
    expect(useComputerPreview.getState().dismissedSession).toBe('s1')

    preview.show('s1')
    expect(useComputerPreview.getState().session).toBe('s1')
    expect(useComputerPreview.getState().dismissedSession).toBeNull()
  })

  it('opens again after sharing is turned off and back on', () => {
    const preview = useComputerPreview.getState()
    preview.dismiss('s1')
    preview.syncController({ session: 's1', enabled: false })
    preview.syncController({ session: 's1', enabled: true })
    expect(useComputerPreview.getState().session).toBe('s1')
    expect(useComputerPreview.getState().dismissedSession).toBeNull()
  })
})
