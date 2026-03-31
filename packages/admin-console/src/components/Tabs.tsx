import { ReactNode } from 'react'

export interface Tab {
  id: string
  label: string
  icon?: React.ComponentType<{ className?: string }>
}

interface TabsProps {
  tabs: Tab[]
  activeTab: string
  onChange: (tabId: string) => void
  children: ReactNode
}

export default function Tabs({ tabs, activeTab, onChange, children }: TabsProps) {
  return (
    <div>
      <div className="flex gap-2 mb-6 border-b border-white/10 select-none">
        {tabs.map((tab) => {
          const Icon = tab.icon
          return (
            <button
              key={tab.id}
              onClick={() => onChange(tab.id)}
              className={`flex items-center gap-2 px-4 py-3 font-medium transition-all ${
                activeTab === tab.id
                  ? 'text-purple-400 border-b-2 border-purple-400'
                  : 'text-gray-400 hover:text-gray-300'
              }`}
            >
              {Icon && <Icon className="w-4 h-4" />}
              {tab.label}
            </button>
          )
        })}
      </div>
      <div>{children}</div>
    </div>
  )
}
