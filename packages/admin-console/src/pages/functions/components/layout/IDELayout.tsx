/**
 * IDE Layout Component
 *
 * VS Code-like layout with resizable panels using Allotment.
 * Layout: [Sidebar | Editor | Properties] / Output
 */

import { type ReactNode } from 'react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'
import { useFunctionsContext } from '../../hooks'

interface IDELayoutProps {
  sidebar: ReactNode
  editor: ReactNode
  properties: ReactNode
  output: ReactNode
}

export function IDELayout({ sidebar, editor, properties, output }: IDELayoutProps) {
  const {
    repo,
    branch,
    preferences,
    setSidebarWidth,
    setPropertiesWidth,
    setOutputHeight,
  } = useFunctionsContext()

  // Only remount layout when repo/branch change, not on every node selection.
  // const allotmentKey = `${repo}:${branch}`
  const allotmentKey = `functions-ide-layout`
  console.log('IDELayout render', allotmentKey, repo, branch)
  return (
    <div className="h-full flex flex-col bg-gradient-to-br from-zinc-900 via-primary-950/20 to-black">
      <Allotment
        key={allotmentKey}
        vertical
        onChange={(sizes) => {
          if (sizes.length >= 2) {
            // Output is the second pane
            setOutputHeight(sizes[1])
          }
        }}
      >
        {/* Main content area (horizontal split) */}
        <Allotment.Pane minSize={200}>
          <Allotment
            onChange={(sizes) => {
              // Update widths based on which panels are visible
              let sidebarIdx = -1
              let propertiesIdx = -1
              let idx = 0

              if (preferences.sidebarVisible) {
                sidebarIdx = idx++
              }
              idx++ // editor is always visible
              if (preferences.propertiesVisible) {
                propertiesIdx = idx
              }

              if (sidebarIdx >= 0 && sizes[sidebarIdx] !== undefined) {
                setSidebarWidth(sizes[sidebarIdx])
              }
              if (propertiesIdx >= 0 && sizes[propertiesIdx] !== undefined) {
                setPropertiesWidth(sizes[propertiesIdx])
              }
            }}
          >
            {/* Sidebar - Function Explorer */}
            {preferences.sidebarVisible && (
              <Allotment.Pane
                preferredSize={preferences.sidebarWidth}
                minSize={200}
                maxSize={500}
              >
                <div className="h-full bg-black/30 backdrop-blur-md border-r border-white/10">
                  {sidebar}
                </div>
              </Allotment.Pane>
            )}

            {/* Editor Area */}
            <Allotment.Pane minSize={300}>
              <div className="h-full bg-[#1a1a2e]">
                {editor}
              </div>
            </Allotment.Pane>

            {/* Properties Panel */}
            {preferences.propertiesVisible && (
              <Allotment.Pane
                preferredSize={preferences.propertiesWidth}
                minSize={200}
                maxSize={500}
              >
                <div className="h-full bg-black/30 backdrop-blur-md border-l border-white/10">
                  {properties}
                </div>
              </Allotment.Pane>
            )}
          </Allotment>
        </Allotment.Pane>

        {/* Output Panel */}
        {preferences.outputVisible && (
          <Allotment.Pane
            preferredSize={preferences.outputHeight}
            minSize={100}
            maxSize={500}
          >
            <div className="h-full bg-black/40 backdrop-blur-md border-t border-white/10">
              {output}
            </div>
          </Allotment.Pane>
        )}
      </Allotment>
    </div>
  )
}
