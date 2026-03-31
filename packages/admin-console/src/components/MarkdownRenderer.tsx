import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter'
import { oneDark } from 'react-syntax-highlighter/dist/esm/styles/prism'
import type { Components } from 'react-markdown'

interface MarkdownRendererProps {
  content: string
  /** Base URL for rewriting relative image paths */
  assetBaseUrl?: string
}

/**
 * Renders markdown content with support for:
 * - GitHub-flavored markdown (tables, strikethrough, task lists, etc.)
 * - Syntax highlighting for code blocks
 * - Relative image path rewriting to API URLs
 * - Dark theme styling
 */
export default function MarkdownRenderer({ content, assetBaseUrl }: MarkdownRendererProps) {
  const components: Components = {
    // Custom image renderer to rewrite relative paths
    img: ({ src, alt, ...props }) => {
      let imageSrc = src || ''

      // Rewrite relative paths (e.g., "static/logo.png" or "./static/logo.png")
      if (assetBaseUrl && src && !src.startsWith('http') && !src.startsWith('data:')) {
        // Remove leading ./ if present
        const cleanPath = src.replace(/^\.\//, '')
        imageSrc = `${assetBaseUrl}/${cleanPath}@file`
      }

      return (
        <img
          src={imageSrc}
          alt={alt || ''}
          className="max-w-full h-auto rounded-lg my-4"
          {...props}
        />
      )
    },
    // Code blocks with syntax highlighting
    code: ({ className, children, ...props }) => {
      const match = /language-(\w+)/.exec(className || '')
      const language = match ? match[1] : ''
      const codeString = String(children).replace(/\n$/, '')

      // Check if this is a code block (has language) or inline code
      const isCodeBlock = match || (codeString.includes('\n'))

      if (isCodeBlock) {
        return (
          <SyntaxHighlighter
            style={oneDark}
            language={language || 'text'}
            PreTag="div"
            customStyle={{
              margin: 0,
              padding: '1rem',
              borderRadius: '0.5rem',
              fontSize: '0.875rem',
              backgroundColor: 'rgba(0, 0, 0, 0.4)',
            }}
            codeTagProps={{
              style: {
                fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
              }
            }}
          >
            {codeString}
          </SyntaxHighlighter>
        )
      }

      // Inline code
      return (
        <code
          className="bg-white/10 px-1.5 py-0.5 rounded text-sm font-mono text-primary-300"
          {...props}
        >
          {children}
        </code>
      )
    },
    // Style pre blocks (code block wrapper) - let SyntaxHighlighter handle styling
    pre: ({ children }) => (
      <div className="my-4 overflow-hidden rounded-lg">
        {children}
      </div>
    ),
    // Style links
    a: ({ href, children, ...props }) => (
      <a
        href={href}
        className="text-primary-400 hover:text-primary-300 underline"
        target="_blank"
        rel="noopener noreferrer"
        {...props}
      >
        {children}
      </a>
    ),
    // Style headings
    h1: ({ children }) => <h1 className="text-2xl font-bold text-white mb-4 mt-6 first:mt-0">{children}</h1>,
    h2: ({ children }) => <h2 className="text-xl font-semibold text-white mb-3 mt-5">{children}</h2>,
    h3: ({ children }) => <h3 className="text-lg font-medium text-white mb-2 mt-4">{children}</h3>,
    h4: ({ children }) => <h4 className="text-base font-medium text-white mb-2 mt-3">{children}</h4>,
    h5: ({ children }) => <h5 className="text-sm font-medium text-white mb-1 mt-2">{children}</h5>,
    h6: ({ children }) => <h6 className="text-sm font-medium text-zinc-300 mb-1 mt-2">{children}</h6>,
    // Style paragraphs
    p: ({ children }) => <p className="text-zinc-300 mb-4 leading-relaxed">{children}</p>,
    // Style lists
    ul: ({ children }) => <ul className="list-disc list-inside text-zinc-300 mb-4 space-y-1 pl-2">{children}</ul>,
    ol: ({ children }) => <ol className="list-decimal list-inside text-zinc-300 mb-4 space-y-1 pl-2">{children}</ol>,
    li: ({ children }) => <li className="text-zinc-300">{children}</li>,
    // Style blockquotes
    blockquote: ({ children }) => (
      <blockquote className="border-l-4 border-primary-500 pl-4 py-2 my-4 text-zinc-400 italic bg-white/5 rounded-r-lg">
        {children}
      </blockquote>
    ),
    // Style tables (GFM)
    table: ({ children }) => (
      <div className="overflow-x-auto my-4 rounded-lg border border-white/10">
        <table className="min-w-full divide-y divide-white/10">{children}</table>
      </div>
    ),
    thead: ({ children }) => <thead className="bg-white/5">{children}</thead>,
    tbody: ({ children }) => <tbody className="divide-y divide-white/5">{children}</tbody>,
    tr: ({ children }) => <tr>{children}</tr>,
    th: ({ children }) => (
      <th className="px-4 py-2 text-left text-sm font-medium text-white">{children}</th>
    ),
    td: ({ children }) => (
      <td className="px-4 py-2 text-sm text-zinc-300">{children}</td>
    ),
    // Style horizontal rules
    hr: () => <hr className="my-6 border-white/10" />,
    // Style strong/bold
    strong: ({ children }) => <strong className="font-semibold text-white">{children}</strong>,
    // Style emphasis/italic
    em: ({ children }) => <em className="italic text-zinc-200">{children}</em>,
    // Style strikethrough (GFM)
    del: ({ children }) => <del className="line-through text-zinc-500">{children}</del>,
  }

  return (
    <div className="markdown-content">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {content}
      </ReactMarkdown>
    </div>
  )
}
