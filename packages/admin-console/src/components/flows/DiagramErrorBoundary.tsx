/**
 * DiagramErrorBoundary
 *
 * Isolates render failures in the flow diagram (e.g. a malformed
 * flow_definition_snapshot) so a crash shows an inline fallback instead of
 * blanking the entire admin page.
 */

import { Component, type ReactNode } from 'react'
import { AlertTriangle } from 'lucide-react'

interface Props {
  children: ReactNode
}

interface State {
  error: Error | null
}

export default class DiagramErrorBoundary extends Component<Props, State> {
  state: State = { error: null }

  static getDerivedStateFromError(error: Error): State {
    return { error }
  }

  componentDidCatch(error: Error) {
    // Surface for debugging without taking down the page
    console.error('[FlowInstanceDiagram] render failed:', error)
  }

  render() {
    if (this.state.error) {
      return (
        <div className="h-32 flex flex-col items-center justify-center gap-2 text-sm text-zinc-400">
          <AlertTriangle className="w-5 h-5 text-yellow-400" />
          <span>Couldn’t render this flow diagram.</span>
          <span className="text-xs text-zinc-500 font-mono">{this.state.error.message}</span>
        </div>
      )
    }
    return this.props.children
  }
}
