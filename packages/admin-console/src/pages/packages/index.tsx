import { Routes, Route } from 'react-router-dom'
import PackagesList from './PackagesList'
import PackageBrowser from './PackageBrowser'
import PackageUpload from './PackageUpload'
import PackageSync from './PackageSync'
import PackageCreate from './PackageCreate'

/**
 * Packages Router Component
 *
 * Handles routing for the package manager with folder support:
 * - /packages - List packages at root
 * - /packages/upload - Upload new package (to current folder)
 * - /packages/create - Create new package from content selection
 * - /packages/folder/subfolder - Navigate folder structure
 * - /packages/folder/package-name - Package details (when node is raisin:Package)
 * - /packages/folder/package-name/browse - Browse package contents
 * - /packages/folder/package-name/sync - Sync status dashboard
 *
 * The PackagesList component handles folder navigation and distinguishes
 * between folders and packages based on node type.
 */
export default function PackagesRouter() {
  return (
    <Routes>
      {/* Upload route - must be before wildcard */}
      <Route path="upload" element={<PackageUpload />} />
      {/* Create route - create new package from content selection */}
      <Route path="create" element={<PackageCreate />} />
      {/* Browse route - package-name/browse */}
      <Route path=":name/browse" element={<PackageBrowser />} />
      {/* Sync route - package-name/sync */}
      <Route path=":name/sync" element={<PackageSync />} />
      {/* Main content route - handles both folders and package details */}
      <Route path="*" element={<PackagesList />} />
      {/* Root route */}
      <Route index element={<PackagesList />} />
    </Routes>
  )
}
