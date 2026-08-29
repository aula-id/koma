import { Component, type ErrorInfo, type ReactNode } from 'react'
import { Empty } from './panels/helpers'
import { logError } from '../utils/logError'

interface Props {
  children: ReactNode
  label?: string
}

interface State {
  hasError: boolean
  error: Error | null
}

export class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props)
    this.state = { hasError: false, error: null }
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    const label = this.props.label ?? 'ErrorBoundary'
    const stack = errorInfo.componentStack?.trim()
    const detail = stack
      ? `${error.message}\n${error.stack ?? ''}\ncomponentStack:${stack}`
      : error
    logError(label, detail)
    console.error(`[${label}]`, error, errorInfo)
  }

  render() {
    if (this.state.hasError) {
      const errorMsg = this.state.error?.message ?? 'Unknown error'
      const errorStack = this.state.error?.stack ?? ''
      return (
        <div className="flex h-full flex-col overflow-hidden p-3">
          <div className="text-koma-error text-xs font-mono overflow-auto">
            <div className="font-bold mb-2">Something went wrong</div>
            <div className="mb-2 opacity-90">{errorMsg}</div>
            {errorStack && (
              <details className="opacity-60">
                <summary className="cursor-pointer hover:opacity-80">Stack trace</summary>
                <pre className="mt-1 whitespace-pre-wrap text-[10px] opacity-80">{errorStack}</pre>
              </details>
            )}
            <div className="mt-2 opacity-50 text-[10px]">Check ~/.koma/error.log for details</div>
          </div>
        </div>
      )
    }

    return this.props.children
  }
}
