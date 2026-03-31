/**
 * Functions IDE Page
 *
 * VS Code-like IDE for managing RaisinDB serverless functions.
 * Route: /admin/{repo}/functions
 */

import { PanelLeftClose, PanelRightClose, PanelBottomClose, PanelLeft, PanelRight, PanelBottom } from 'lucide-react'
import { FunctionsProvider, useFunctionsContext } from './hooks'
import { IDELayout, FunctionExplorer, EditorPane, PropertiesPanel, OutputPanel } from './components'

function FunctionsIDEContent() {
  const {
    preferences,
    toggleSidebar,
    toggleProperties,
    toggleOutput,
  } = useFunctionsContext()

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="flex-shrink-0 bg-black/30 backdrop-blur-md border-b border-white/10">
        <div className="px-4 py-2 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <span className="text-white font-semibold">Functions</span>
          </div>

          <div className="flex items-center gap-2">
            {/* Panel toggles */}
            <button
              onClick={toggleSidebar}
              className={`p-1.5 rounded ${preferences.sidebarVisible ? 'text-white bg-white/10' : 'text-gray-500 hover:text-white'}`}
              title={preferences.sidebarVisible ? 'Hide sidebar' : 'Show sidebar'}
            >
              {preferences.sidebarVisible ? <PanelLeftClose className="w-4 h-4" /> : <PanelLeft className="w-4 h-4" />}
            </button>

            <button
              onClick={toggleProperties}
              className={`p-1.5 rounded ${preferences.propertiesVisible ? 'text-white bg-white/10' : 'text-gray-500 hover:text-white'}`}
              title={preferences.propertiesVisible ? 'Hide properties' : 'Show properties'}
            >
              {preferences.propertiesVisible ? <PanelRightClose className="w-4 h-4" /> : <PanelRight className="w-4 h-4" />}
            </button>

            <button
              onClick={toggleOutput}
              className={`p-1.5 rounded ${preferences.outputVisible ? 'text-white bg-white/10' : 'text-gray-500 hover:text-white'}`}
              title={preferences.outputVisible ? 'Hide output' : 'Show output'}
            >
              {preferences.outputVisible ? <PanelBottomClose className="w-4 h-4" /> : <PanelBottom className="w-4 h-4" />}
            </button>
          </div>
        </div>
      </div>

      {/* IDE Layout */}
      <div className="flex-1 min-h-0">
        <IDELayout
          sidebar={<FunctionExplorer />}
          editor={<EditorPane />}
          properties={<PropertiesPanel />}
          output={<OutputPanel />}
        />
      </div>
    </div>
  )
}

export default function FunctionsIDE() {
  return (
    <FunctionsProvider>
      <FunctionsIDEContent />
    </FunctionsProvider>
  )
}
