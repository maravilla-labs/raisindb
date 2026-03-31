import { useEffect, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import * as LucideIcons from 'lucide-react'
import { Package, Upload, Search, Filter, CheckCircle, XCircle, Download, Sparkles, RefreshCw, FolderPlus } from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import Breadcrumb from '../../components/Breadcrumb'
import FolderCard from '../../components/FolderCard'
import FolderDialog from '../../components/FolderDialog'
import { useToast, ToastContainer } from '../../components/Toast'
import { nodesApi, type Node } from '../../api/nodes'
import { requestRaw } from '../../api/client'
import PackageDetails from './PackageDetails'

const WORKSPACE = 'packages'

// Types
interface PackageNode {
  id: string
  name: string
  path: string
  version: string
  title?: string
  description?: string
  author?: string
  installed: boolean
  category?: string
  keywords?: string[]
  icon?: string
  color?: string
  upload_state?: 'new' | 'updated'
  teaser_background_url?: string
}

// Helper to check if icon is a URL
const isIconUrl = (icon: string): boolean => {
  return icon.startsWith('http://') || icon.startsWith('https://') || icon.startsWith('/')
}

// Build full URL for teaser background from relative path
const buildTeaserUrl = (repo: string, branch: string, packagePath: string, relativePath: string): string => {
  // Build API URL using package path
  const encodedPath = packagePath.split('/').map(encodeURIComponent).join('/')
  return `/api/repository/${repo}/${branch}/head/packages${encodedPath}/${relativePath}@file`
}

// Fetch image with auth and return blob URL
async function fetchTeaserImage(url: string): Promise<string | null> {
  try {
    const response = await requestRaw(url)
    const blob = await response.blob()
    return URL.createObjectURL(blob)
  } catch (error) {
    console.error('Failed to fetch teaser image:', error)
    return null
  }
}

// Convert kebab-case to PascalCase for Lucide component lookup
const iconNameToPascalCase = (name: string): string => {
  return name
    .split('-')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join('')
}

// Get Lucide icon component by name
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const getIconComponent = (name: string): any => {
  const pascalName = iconNameToPascalCase(name)
  return (LucideIcons as any)[pascalName] || Package
}

// Convert Node to PackageNode
function nodeToPackage(node: Node): PackageNode {
  return {
    id: node.id,
    name: node.name,
    path: node.path,
    version: node.properties?.version as string || '0.0.0',
    title: node.properties?.title as string,
    description: node.properties?.description as string,
    author: node.properties?.author as string,
    installed: node.properties?.installed as boolean || false,
    category: node.properties?.category as string,
    keywords: node.properties?.keywords as string[],
    icon: node.properties?.icon as string,
    color: node.properties?.color as string,
    upload_state: node.properties?.upload_state as 'new' | 'updated',
    teaser_background_url: node.properties?.teaser_background_url as string,
  }
}

type FilterType = 'all' | 'installed' | 'not-installed'

export default function PackagesList() {
  const navigate = useNavigate()
  const { repo, branch, '*': pathParam } = useParams<{ repo: string; branch?: string; '*': string }>()
  const activeBranch = branch || 'main'

  // Compute current path from URL
  // pathParam might be empty, 'myfolder', 'myfolder/subfolder', etc.
  const currentPath = pathParam ? `/${pathParam}` : ''

  const [folders, setFolders] = useState<Node[]>([])
  const [packages, setPackages] = useState<PackageNode[]>([])
  const [loading, setLoading] = useState(true)
  const [filter, setFilter] = useState<FilterType>('all')
  const [searchQuery, setSearchQuery] = useState('')
  const [showFolderDialog, setShowFolderDialog] = useState(false)
  const [editingFolder, setEditingFolder] = useState<Node | undefined>(undefined)
  const [isPackageNode, setIsPackageNode] = useState(false)
  const [teaserImages, setTeaserImages] = useState<Record<string, string>>({})
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  useEffect(() => {
    loadContent()
  }, [repo, activeBranch, currentPath])

  // Load teaser images with auth when packages change
  useEffect(() => {
    if (!repo || packages.length === 0) return

    const loadTeaserImages = async () => {
      const newImages: Record<string, string> = {}

      for (const pkg of packages) {
        if (pkg.teaser_background_url) {
          const url = buildTeaserUrl(repo, activeBranch, pkg.path, pkg.teaser_background_url)
          const blobUrl = await fetchTeaserImage(url)
          if (blobUrl) {
            newImages[pkg.id] = blobUrl
          }
        }
      }

      setTeaserImages(prev => {
        // Revoke old URLs to prevent memory leaks
        Object.values(prev).forEach(url => URL.revokeObjectURL(url))
        return newImages
      })
    }

    loadTeaserImages()

    // Cleanup on unmount
    return () => {
      Object.values(teaserImages).forEach(url => URL.revokeObjectURL(url))
    }
  }, [packages, repo, activeBranch])

  async function loadContent() {
    if (!repo) return
    setLoading(true)
    setIsPackageNode(false)

    try {
      // Check if current path points to a Package node (not a folder)
      if (currentPath) {
        try {
          const targetNode = await nodesApi.getAtHead(repo, activeBranch, WORKSPACE, currentPath)
          if (targetNode && targetNode.node_type === 'raisin:Package') {
            // Current path is a Package node - render details instead of list
            setIsPackageNode(true)
            setLoading(false)
            return
          }
        } catch {
          // Node doesn't exist or can't be loaded - continue with folder loading
        }
      }

      // Load children at current path
      const nodes = await nodesApi.listChildrenAtHead(repo, activeBranch, WORKSPACE, currentPath || '/')

      // Separate folders and packages
      const folderNodes = nodes.filter(n => n.node_type === 'raisin:Folder')
      const packageNodes = nodes.filter(n => n.node_type === 'raisin:Package')

      setFolders(folderNodes)
      setPackages(packageNodes.map(nodeToPackage))
    } catch (error) {
      console.error('Failed to load content:', error)
      showError('Load Failed', 'Failed to load packages')
    } finally {
      setLoading(false)
    }
  }

  function navigateToPath(path: string) {
    if (!repo) return
    if (!path || path === '/') {
      navigate(`/${repo}/${activeBranch}/packages`)
      return
    }
    // Remove leading slash for URL
    const urlPath = path.startsWith('/') ? path.slice(1) : path
    navigate(`/${repo}/${activeBranch}/packages/${urlPath}`)
  }

  function handleFolderClick(folder: Node) {
    navigateToPath(folder.path)
  }

  function handlePackageClick(pkg: PackageNode) {
    navigateToPath(pkg.path)
  }

  function handleNavigate(path: string) {
    navigateToPath(path)
  }

  function handleUploadClick() {
    // Pass current path and branch to upload page via state
    navigate(`/${repo}/${activeBranch}/packages/upload`, { state: { currentPath, activeBranch } })
  }

  async function handleSaveFolder(data: { name: string; description: string; icon: string; color: string }) {
    if (!repo) return

    const properties = {
      description: data.description,
      icon: data.icon,
      color: data.color,
    }

    try {
      if (editingFolder) {
        // Update existing folder
        await nodesApi.update(repo, activeBranch, WORKSPACE, editingFolder.path, {
          properties,
        })
        showSuccess('Folder Updated', `Folder "${data.name}" has been updated`)
      } else {
        // Create new folder under current path
        // nodesApi.create expects: (repo, branch, workspace, parentPath, request)
        const parentPath = currentPath || '/'
        await nodesApi.create(repo, activeBranch, WORKSPACE, parentPath, {
          name: data.name,
          node_type: 'raisin:Folder',
          properties,
        })
        showSuccess('Folder Created', `Folder "${data.name}" has been created`)
      }
      setShowFolderDialog(false)
      setEditingFolder(undefined)
      loadContent()
    } catch (error) {
      console.error('Failed to save folder:', error)
      showError('Save Failed', 'Failed to save folder')
    }
  }

  // If current path is a Package node, render PackageDetails
  if (isPackageNode) {
    return <PackageDetails />
  }

  // Filter packages
  const filteredPackages = packages.filter(pkg => {
    // Apply installed filter
    if (filter === 'installed' && !pkg.installed) return false
    if (filter === 'not-installed' && pkg.installed) return false

    // Apply search filter
    if (!searchQuery) return true
    const query = searchQuery.toLowerCase()
    return (
      pkg.name.toLowerCase().includes(query) ||
      pkg.title?.toLowerCase().includes(query) ||
      pkg.description?.toLowerCase().includes(query) ||
      pkg.keywords?.some(k => k.toLowerCase().includes(query))
    )
  })

  // Build breadcrumb segments from current path
  const buildBreadcrumbSegments = () => {
    if (!currentPath || currentPath === '/') return []

    const parts = currentPath.split('/').filter(Boolean)
    return parts.map((part, index) => ({
      label: part,
      path: '/' + parts.slice(0, index + 1).join('/'),
    }))
  }

  return (
    <div className="animate-fade-in">
      {/* Header with breadcrumb */}
      <div className="mb-6">
        <Breadcrumb
          segments={buildBreadcrumbSegments()}
          onNavigate={handleNavigate}
        />
      </div>

      <div className="mb-8 flex justify-between items-start">
        <div>
          <h1 className="text-4xl font-bold text-white mb-2">
            {currentPath ? currentPath.split('/').pop() : 'Packages'}
          </h1>
          <p className="text-zinc-400">Browse and manage RaisinDB packages</p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => setShowFolderDialog(true)}
            className="flex items-center gap-2 px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
          >
            <FolderPlus className="w-5 h-5" />
            New Folder
          </button>
          <button
            onClick={handleUploadClick}
            className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
          >
            <Upload className="w-5 h-5" />
            Upload Package
          </button>
        </div>
      </div>

      {/* Search and Filter */}
      <div className="mb-6 flex gap-4 items-center">
        <div className="flex-1 relative">
          <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-5 h-5 text-zinc-400" />
          <input
            type="text"
            placeholder="Search packages by name, description, or keywords..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="w-full pl-10 pr-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white placeholder-zinc-400 focus:outline-none focus:ring-2 focus:ring-primary-500"
          />
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => setFilter('all')}
            className={`flex items-center gap-2 px-4 py-2 rounded-lg transition-all ${
              filter === 'all'
                ? 'bg-primary-500 text-white'
                : 'bg-white/10 text-zinc-400 hover:bg-white/20'
            }`}
          >
            <Filter className="w-4 h-4" />
            All
          </button>
          <button
            onClick={() => setFilter('installed')}
            className={`flex items-center gap-2 px-4 py-2 rounded-lg transition-all ${
              filter === 'installed'
                ? 'bg-primary-500 text-white'
                : 'bg-white/10 text-zinc-400 hover:bg-white/20'
            }`}
          >
            <CheckCircle className="w-4 h-4" />
            Installed
          </button>
          <button
            onClick={() => setFilter('not-installed')}
            className={`flex items-center gap-2 px-4 py-2 rounded-lg transition-all ${
              filter === 'not-installed'
                ? 'bg-primary-500 text-white'
                : 'bg-white/10 text-zinc-400 hover:bg-white/20'
            }`}
          >
            <Download className="w-4 h-4" />
            Not Installed
          </button>
        </div>
      </div>

      {loading ? (
        <div className="text-center text-zinc-400 py-12">Loading...</div>
      ) : folders.length === 0 && filteredPackages.length === 0 ? (
        <GlassCard>
          <div className="text-center py-12">
            <Package className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">
              {searchQuery
                ? 'No packages found'
                : filter === 'installed'
                ? 'No installed packages'
                : filter === 'not-installed'
                ? 'All packages are installed'
                : 'No packages yet'}
            </h3>
            <p className="text-zinc-400">
              {searchQuery
                ? 'Try adjusting your search query'
                : filter === 'all'
                ? 'Upload a package to get started'
                : 'Install packages to extend RaisinDB functionality'}
            </p>
          </div>
        </GlassCard>
      ) : (
        <>
          {/* Folders */}
          {folders.length > 0 && (
            <div className="mb-6">
              <h2 className="text-lg font-semibold text-white mb-3">Folders</h2>
              <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-6 gap-3">
                {folders.map((folder) => (
                  <FolderCard
                    key={folder.id}
                    folder={folder}
                    onClick={() => handleFolderClick(folder)}
                  />
                ))}
              </div>
            </div>
          )}

          {/* Packages */}
          {filteredPackages.length > 0 && (
            <div>
              {folders.length > 0 && (
                <h2 className="text-lg font-semibold text-white mb-3">Packages</h2>
              )}
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                {filteredPackages.map((pkg) => {
                  const teaserUrl = teaserImages[pkg.id]

                  return (
                    <div
                      key={pkg.id}
                      onClick={() => handlePackageClick(pkg)}
                      className="relative overflow-hidden rounded-xl cursor-pointer group transition-all duration-200 hover:scale-[1.02]"
                    >
                      {/* Teaser background image if available */}
                      {teaserUrl && (
                        <div
                          className="absolute inset-0 bg-cover bg-center transition-transform duration-300 group-hover:scale-105"
                          style={{ backgroundImage: `url(${teaserUrl})` }}
                        />
                      )}
                      {/* Gradient overlay for readability */}
                      <div className={`absolute inset-0 ${teaserUrl ? 'bg-gradient-to-t from-black/90 via-black/70 to-black/40' : 'bg-white/5'}`} />

                      {/* Card content */}
                      <div className="relative p-5">
                        <div className="flex items-start gap-4">
                          {pkg.icon && isIconUrl(pkg.icon) ? (
                            // URL-based icon - render as image
                            <div className="w-12 h-12 flex-shrink-0 rounded-lg overflow-hidden bg-white/10">
                              <img src={pkg.icon} alt={pkg.name} className="w-full h-full object-cover" />
                            </div>
                          ) : pkg.color ? (
                            // Custom color provided - use inline styles
                            (() => {
                              const IconComponent = pkg.icon ? getIconComponent(pkg.icon) : Package
                              return (
                                <div
                                  className="w-12 h-12 flex-shrink-0 rounded-lg flex items-center justify-center backdrop-blur-sm"
                                  style={{ backgroundColor: `${pkg.color}30` }}
                                >
                                  <IconComponent className="w-6 h-6" style={{ color: pkg.color }} />
                                </div>
                              )
                            })()
                          ) : (
                            // Default - use Tailwind primary colors
                            (() => {
                              const IconComponent = pkg.icon ? getIconComponent(pkg.icon) : Package
                              return (
                                <div className="w-12 h-12 flex-shrink-0 rounded-lg bg-primary-500/30 backdrop-blur-sm flex items-center justify-center">
                                  <IconComponent className="w-6 h-6 text-primary-400" />
                                </div>
                              )
                            })()
                          )}
                          <div className="flex-1 min-w-0">
                            <div className="flex items-start justify-between gap-2 mb-1">
                              <h3 className="text-lg font-semibold text-white truncate">
                                {pkg.title || pkg.name}
                              </h3>
                              {/* Status badges */}
                              <div className="flex gap-1.5 flex-shrink-0">
                                {/* Upload state badge (New/Updated) */}
                                {pkg.upload_state === 'new' && !pkg.installed && (
                                  <span className="flex items-center gap-1 px-2 py-0.5 bg-blue-500/20 text-blue-400 text-xs rounded-full">
                                    <Sparkles className="w-3 h-3" />
                                    New
                                  </span>
                                )}
                                {pkg.upload_state === 'updated' && !pkg.installed && (
                                  <span className="flex items-center gap-1 px-2 py-0.5 bg-amber-500/20 text-amber-400 text-xs rounded-full">
                                    <RefreshCw className="w-3 h-3" />
                                    Updated
                                  </span>
                                )}
                                {/* Installed badge */}
                                {pkg.installed ? (
                                  <span className="flex items-center gap-1 px-2 py-0.5 bg-green-500/20 text-green-400 text-xs rounded-full">
                                    <CheckCircle className="w-3 h-3" />
                                    Installed
                                  </span>
                                ) : !pkg.upload_state && (
                                  <span className="flex items-center gap-1 px-2 py-0.5 bg-gray-500/20 text-zinc-400 text-xs rounded-full">
                                    <XCircle className="w-3 h-3" />
                                    Not Installed
                                  </span>
                                )}
                              </div>
                            </div>
                            <p className="text-xs text-zinc-400 mb-2">v{pkg.version}</p>
                            {pkg.description && (
                              <p className="text-sm text-zinc-300 line-clamp-2 mb-2">
                                {pkg.description}
                              </p>
                            )}
                            {pkg.author && (
                              <p className="text-xs text-zinc-400">by {pkg.author}</p>
                            )}
                            {pkg.keywords && pkg.keywords.length > 0 && (
                              <div className="flex flex-wrap gap-1 mt-2">
                                {pkg.keywords.slice(0, 3).map((keyword) => (
                                  <span
                                    key={keyword}
                                    className="px-2 py-0.5 bg-white/10 text-zinc-300 text-xs rounded"
                                  >
                                    {keyword}
                                  </span>
                                ))}
                                {pkg.keywords.length > 3 && (
                                  <span className="px-2 py-0.5 text-zinc-400 text-xs">
                                    +{pkg.keywords.length - 3}
                                  </span>
                                )}
                              </div>
                            )}
                          </div>
                        </div>
                      </div>
                    </div>
                  )
                })}
              </div>
            </div>
          )}
        </>
      )}

      {/* Folder Dialog */}
      {showFolderDialog && (
        <FolderDialog
          onClose={() => {
            setShowFolderDialog(false)
            setEditingFolder(undefined)
          }}
          onSave={handleSaveFolder}
          folder={editingFolder}
        />
      )}

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
