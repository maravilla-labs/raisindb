import { useEffect, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import {
  ArrowLeft,
  Folder,
  File,
  FileCode,
  FileJson,
  FileText,
  FileImage,
  ChevronRight,
  Home,
} from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import { useToast, ToastContainer } from '../../components/Toast'
import { packagesApi, type PackageFile } from '../../api/packages'
import { requestRaw } from '../../api/client'

// Image extensions that should be previewed as images
const IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'ico', 'bmp']

function isImageFile(filename: string): boolean {
  const ext = filename.split('.').pop()?.toLowerCase()
  return ext ? IMAGE_EXTENSIONS.includes(ext) : false
}

export default function PackageBrowser() {
  const navigate = useNavigate()
  const { repo, name } = useParams<{ repo: string; name: string }>()
  const [currentPath, setCurrentPath] = useState('')
  const [files, setFiles] = useState<PackageFile[]>([])
  const [fileContent, setFileContent] = useState<string | null>(null)
  const [imageUrl, setImageUrl] = useState<string | null>(null)
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const { toasts, error: showError, closeToast } = useToast()

  useEffect(() => {
    loadFiles(currentPath)
  }, [repo, name, currentPath])

  // Clean up blob URL when component unmounts or imageUrl changes
  useEffect(() => {
    return () => {
      if (imageUrl) {
        URL.revokeObjectURL(imageUrl)
      }
    }
  }, [imageUrl])

  async function loadFiles(path: string) {
    if (!repo || !name) return
    setLoading(true)
    try {
      const data = await packagesApi.browsePackageContents(
        repo,
        decodeURIComponent(name),
        path
      )
      setFiles(data)
    } catch (error) {
      console.error('Failed to load package contents:', error)
      showError('Load Failed', 'Failed to load package contents')
    } finally {
      setLoading(false)
    }
  }

  async function handleFileClick(file: PackageFile) {
    if (file.type === 'directory') {
      const newPath = currentPath ? `${currentPath}/${file.name}` : file.name
      setCurrentPath(newPath)
      setFileContent(null)
      setImageUrl(null)
      setSelectedFile(null)
    } else {
      // Load file content
      if (!repo || !name) return
      try {
        const filePath = currentPath ? `${currentPath}/${file.name}` : file.name

        // Clear previous content
        setFileContent(null)
        setImageUrl(null)
        setSelectedFile(filePath)

        if (isImageFile(file.name)) {
          // Fetch image as blob and create object URL
          const response = await requestRaw(
            `/api/packages/${repo}/main/head/${encodeURIComponent(decodeURIComponent(name))}/raisin:file/${filePath}`
          )
          const blob = await response.blob()
          const url = URL.createObjectURL(blob)
          setImageUrl(url)
        } else {
          // Fetch as text
          const content = await packagesApi.getPackageFile(
            repo,
            decodeURIComponent(name),
            filePath
          )
          setFileContent(content)
        }
      } catch (error) {
        console.error('Failed to load file:', error)
        showError('Load Failed', 'Failed to load file content')
      }
    }
  }

  function getFileIcon(file: PackageFile) {
    if (file.type === 'directory') {
      return <Folder className="w-5 h-5 text-primary-400" />
    }

    // Check if it's an image file first
    if (isImageFile(file.name)) {
      return <FileImage className="w-5 h-5 text-pink-400" />
    }

    const ext = file.name.split('.').pop()?.toLowerCase()
    switch (ext) {
      case 'json':
        return <FileJson className="w-5 h-5 text-yellow-400" />
      case 'yaml':
      case 'yml':
        return <FileCode className="w-5 h-5 text-blue-400" />
      case 'js':
      case 'ts':
      case 'jsx':
      case 'tsx':
        return <FileCode className="w-5 h-5 text-green-400" />
      case 'md':
      case 'txt':
        return <FileText className="w-5 h-5 text-zinc-400" />
      default:
        return <File className="w-5 h-5 text-zinc-400" />
    }
  }

  function getLanguageFromFilename(filename: string): string {
    const ext = filename.split('.').pop()?.toLowerCase()
    switch (ext) {
      case 'js':
      case 'jsx':
        return 'javascript'
      case 'ts':
      case 'tsx':
        return 'typescript'
      case 'json':
        return 'json'
      case 'yaml':
      case 'yml':
        return 'yaml'
      case 'md':
        return 'markdown'
      case 'css':
        return 'css'
      case 'html':
        return 'html'
      default:
        return 'plaintext'
    }
  }

  function handleNavigateUp() {
    if (!currentPath) return
    const parts = currentPath.split('/')
    parts.pop()
    setCurrentPath(parts.join('/'))
    setFileContent(null)
    setImageUrl(null)
    setSelectedFile(null)
  }

  function handleNavigateToRoot() {
    setCurrentPath('')
    setFileContent(null)
    setImageUrl(null)
    setSelectedFile(null)
  }

  function handleNavigateToBreadcrumb(index: number) {
    const parts = currentPath.split('/')
    const newPath = parts.slice(0, index + 1).join('/')
    setCurrentPath(newPath)
    setFileContent(null)
    setImageUrl(null)
    setSelectedFile(null)
  }

  const pathParts = currentPath ? currentPath.split('/') : []

  return (
    <div className="animate-fade-in h-full flex flex-col">
      {/* Header */}
      <div className="mb-6 flex-shrink-0">
        <button
          onClick={() => navigate(`/${repo}/packages/${name}`)}
          className="flex items-center gap-2 text-zinc-400 hover:text-white mb-4 transition-colors"
        >
          <ArrowLeft className="w-4 h-4" />
          Back to Package Details
        </button>

        <h1 className="text-3xl font-bold text-white mb-2">Browse Package Contents</h1>
        <p className="text-zinc-400">Explore files in {decodeURIComponent(name || '')}</p>
      </div>

      {/* Breadcrumb */}
      <div className="mb-4 flex-shrink-0">
        <div className="flex items-center gap-2 text-sm">
          <button
            onClick={handleNavigateToRoot}
            className="flex items-center gap-1 px-2 py-1 rounded hover:bg-white/10 text-zinc-400 hover:text-white transition-colors"
          >
            <Home className="w-4 h-4" />
            Root
          </button>
          {pathParts.map((part, index) => (
            <div key={index} className="flex items-center gap-2">
              <ChevronRight className="w-4 h-4 text-zinc-600" />
              <button
                onClick={() => handleNavigateToBreadcrumb(index)}
                className="px-2 py-1 rounded hover:bg-white/10 text-zinc-400 hover:text-white transition-colors"
              >
                {part}
              </button>
            </div>
          ))}
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 grid grid-cols-1 lg:grid-cols-2 gap-6 min-h-0">
        {/* File Tree */}
        <GlassCard className="flex flex-col min-h-0">
          <h2 className="text-xl font-semibold text-white mb-4 flex-shrink-0">
            {currentPath ? currentPath : 'Files'}
          </h2>

          {loading ? (
            <div className="text-center text-zinc-400 py-8">Loading...</div>
          ) : (
            <div className="flex-1 overflow-y-auto space-y-1">
              {currentPath && (
                <button
                  onClick={handleNavigateUp}
                  className="w-full flex items-center gap-3 px-3 py-2 rounded hover:bg-white/10 transition-colors text-left"
                >
                  <Folder className="w-5 h-5 text-zinc-400" />
                  <span className="text-zinc-400">..</span>
                </button>
              )}
              {files.length === 0 ? (
                <div className="text-center text-zinc-500 py-8">
                  No files in this directory
                </div>
              ) : (
                files.map((file) => (
                  <button
                    key={file.path}
                    onClick={() => handleFileClick(file)}
                    className={`w-full flex items-center gap-3 px-3 py-2 rounded hover:bg-white/10 transition-colors text-left ${
                      selectedFile === file.path ? 'bg-primary-500/20' : ''
                    }`}
                  >
                    {getFileIcon(file)}
                    <div className="flex-1 min-w-0">
                      <p className="text-white truncate">{file.name}</p>
                      {file.size !== undefined && file.type === 'file' && (
                        <p className="text-xs text-zinc-500">
                          {(file.size / 1024).toFixed(1)} KB
                        </p>
                      )}
                    </div>
                  </button>
                ))
              )}
            </div>
          )}
        </GlassCard>

        {/* File Preview */}
        <GlassCard className="flex flex-col min-h-0">
          <h2 className="text-xl font-semibold text-white mb-4 flex-shrink-0">
            {selectedFile ? 'Preview' : 'Select a file'}
          </h2>

          <div className="flex-1 overflow-auto">
            {imageUrl ? (
              <div className="h-full">
                <div className="mb-2 text-xs text-zinc-500 flex-shrink-0">
                  {selectedFile}
                </div>
                <div className="bg-black/20 p-4 rounded-lg flex items-center justify-center min-h-[200px]">
                  <img
                    src={imageUrl}
                    alt={selectedFile || 'Preview'}
                    className="max-w-full max-h-[500px] object-contain rounded"
                  />
                </div>
              </div>
            ) : fileContent ? (
              <div className="h-full">
                <div className="mb-2 text-xs text-zinc-500 flex-shrink-0">
                  {selectedFile}
                </div>
                <pre className="bg-black/20 p-4 rounded-lg overflow-auto text-sm text-zinc-300 font-mono h-full">
                  <code className={`language-${getLanguageFromFilename(selectedFile || '')}`}>
                    {fileContent}
                  </code>
                </pre>
              </div>
            ) : (
              <div className="text-center text-zinc-500 py-12">
                <FileCode className="w-12 h-12 mx-auto mb-3 opacity-50" />
                <p>Select a file to preview its contents</p>
              </div>
            )}
          </div>
        </GlassCard>
      </div>

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
