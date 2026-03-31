import Editor from '@monaco-editor/react'

interface YamlEditorProps {
  value: string
  onChange: (value: string | undefined) => void
  readOnly?: boolean
  height?: string
}

export default function YamlEditor({ value, onChange, readOnly = false, height = '500px' }: YamlEditorProps) {
  const isFullHeight = height === '100%'

  return (
    <div className={`overflow-hidden border border-white/20 ${isFullHeight ? 'h-full' : 'rounded-lg'}`}>
      <Editor
        height={height}
        defaultLanguage="yaml"
        value={value}
        onChange={onChange}
        theme="vs-dark"
        options={{
          minimap: { enabled: false },
          fontSize: 14,
          lineNumbers: 'on',
          scrollBeyondLastLine: false,
          automaticLayout: true,
          readOnly,
          wordWrap: 'on',
          folding: true,
          tabSize: 2,
        }}
      />
    </div>
  )
}
