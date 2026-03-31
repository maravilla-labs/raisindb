/**
 * Builder Layout Component
 *
 * Three-panel resizable layout for visual builders using Allotment.
 * Layout: [Toolbox | Canvas | Properties]
 */

import { type ReactNode } from 'react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'

export interface BuilderLayoutPreferences {
  toolboxWidth: number
  propertiesWidth: number
  toolboxVisible: boolean
  propertiesVisible: boolean
}

export const DEFAULT_BUILDER_PREFERENCES: BuilderLayoutPreferences = {
  toolboxWidth: 160,
  propertiesWidth: 320,
  toolboxVisible: true,
  propertiesVisible: true,
}

interface BuilderLayoutProps {
  /** Left panel - type toolbox */
  toolbox: ReactNode
  /** Center panel - main canvas */
  canvas: ReactNode
  /** Right panel - properties editor */
  properties: ReactNode
  /** Current preferences */
  preferences: BuilderLayoutPreferences
  /** Callback to update toolbox width */
  onToolboxWidthChange?: (width: number) => void
  /** Callback to update properties width */
  onPropertiesWidthChange?: (width: number) => void
}

export function BuilderLayout({
  toolbox,
  canvas,
  properties,
  preferences,
  onToolboxWidthChange,
  onPropertiesWidthChange,
}: BuilderLayoutProps) {
  return (
    <div className="h-full flex flex-col">
      <Allotment
        onChange={(sizes) => {
          // Update widths based on which panels are visible
          let toolboxIdx = -1
          let propertiesIdx = -1
          let idx = 0

          if (preferences.toolboxVisible) {
            toolboxIdx = idx++
          }
          idx++ // canvas is always visible
          if (preferences.propertiesVisible) {
            propertiesIdx = idx
          }

          if (toolboxIdx >= 0 && sizes[toolboxIdx] !== undefined) {
            onToolboxWidthChange?.(sizes[toolboxIdx])
          }
          if (propertiesIdx >= 0 && sizes[propertiesIdx] !== undefined) {
            onPropertiesWidthChange?.(sizes[propertiesIdx])
          }
        }}
      >
        {/* Toolbox Panel */}
        {preferences.toolboxVisible && (
          <Allotment.Pane
            preferredSize={preferences.toolboxWidth}
            minSize={120}
            maxSize={300}
          >
            <div className="h-full bg-zinc-900/50 border-r border-white/10 overflow-hidden">
              {toolbox}
            </div>
          </Allotment.Pane>
        )}

        {/* Canvas Panel */}
        <Allotment.Pane minSize={300}>
          <div className="h-full bg-zinc-900/30 overflow-hidden">
            {canvas}
          </div>
        </Allotment.Pane>

        {/* Properties Panel */}
        {preferences.propertiesVisible && (
          <Allotment.Pane
            preferredSize={preferences.propertiesWidth}
            minSize={200}
            maxSize={500}
          >
            <div className="h-full bg-zinc-900/50 border-l border-white/10 overflow-hidden">
              {properties}
            </div>
          </Allotment.Pane>
        )}
      </Allotment>
    </div>
  )
}
