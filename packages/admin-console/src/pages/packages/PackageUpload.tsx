import { useState, useRef, useCallback, useEffect } from 'react'
import { useNavigate, useParams, useLocation } from 'react-router-dom'
import { ArrowLeft, Upload, File, CheckCircle, XCircle, AlertCircle, Loader2, FolderOpen } from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import { useToast, ToastContainer } from '../../components/Toast'
import { packagesApi } from '../../api/packages'
import { sseManager, JobEvent } from '../../api/management'

type UploadStatus = 'idle' | 'uploading' | 'processing' | 'complete' | 'error'

interface LocationState {
  currentPath?: string
  activeBranch?: string
}

export default function PackageUpload() {
  const navigate = useNavigate()
  const location = useLocation()
  const { repo, branch } = useParams<{ repo: string; branch: string }>()

  // Get current path and branch from location state (passed from PackagesList)
  const locationState = location.state as LocationState | null
  const currentPath = locationState?.currentPath || '/'
  const activeBranch = locationState?.activeBranch || branch || 'main'

  const fileInputRef = useRef<HTMLInputElement>(null)
  const [selectedFile, setSelectedFile] = useState<File | null>(null)
  const [uploadStatus, setUploadStatus] = useState<UploadStatus>('idle')
  const [uploadProgress, setUploadProgress] = useState(0)
  const [processingProgress, setProcessingProgress] = useState(0)
  const [statusMessage, setStatusMessage] = useState('')
  const [isDragging, setIsDragging] = useState(false)
  const [trackingJobId, setTrackingJobId] = useState<string | null>(null)
  const [packageName, setPackageName] = useState<string | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  // SSE connection for job progress tracking
  useEffect(() => {
    if (!trackingJobId) return

    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event: JobEvent) => {
        // Only process events for our job
        if (event.job_id !== trackingJobId) return

        // Update progress if available
        if (event.progress !== undefined && event.progress !== null) {
          setProcessingProgress(Math.round(event.progress * 100))
        }

        // Check job status
        if (event.status === 'Completed') {
          setUploadStatus('complete')
          setProcessingProgress(100)
          setStatusMessage('Package processed successfully!')
          showSuccess('Upload Complete', `Package processed successfully`)

          // Navigate to package details after a short delay
          if (packageName) {
            setTimeout(() => {
              // Build the full path to the package
              const packagePath = currentPath === '/' || currentPath === ''
                ? packageName
                : `${currentPath.replace(/^\//, '')}/${packageName}`
              navigate(`/${repo}/${activeBranch}/packages/${packagePath}`)
            }, 1500)
          }
        } else if (event.status.startsWith('Failed')) {
          setUploadStatus('error')
          const errorMsg = event.error || event.status.replace('Failed: ', '')
          setStatusMessage(`Processing failed: ${errorMsg}`)
          showError('Processing Failed', errorMsg)
        } else if (event.status === 'Running') {
          setUploadStatus('processing')
          // Extract progress message from job_type if it's a progress update
          if (event.job_type?.startsWith('Custom(progress:')) {
            const progress = parseFloat(event.job_type.replace('Custom(progress:', '').replace(')', ''))
            if (!isNaN(progress)) {
              setProcessingProgress(Math.round(progress * 100))
            }
          }
        }
      },
      onError: () => {
        console.error('SSE connection error for job tracking')
      }
    })

    return cleanup
  }, [trackingJobId, packageName, repo, activeBranch, currentPath, navigate, showSuccess, showError])

  const handleFileSelect = useCallback((file: File) => {
    // Validate file type
    if (!file.name.endsWith('.rap')) {
      showError('Invalid File', 'Please select a .rap package file')
      return
    }

    setSelectedFile(file)
  }, [showError])

  const handleFileInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (file) {
      handleFileSelect(file)
    }
  }

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    setIsDragging(true)
  }, [])

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    setIsDragging(false)
  }, [])

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault()
    setIsDragging(false)

    const file = e.dataTransfer.files[0]
    if (file) {
      handleFileSelect(file)
    }
  }, [handleFileSelect])

  async function handleUpload() {
    if (!repo || !selectedFile) return

    setUploadStatus('uploading')
    setUploadProgress(0)
    setProcessingProgress(0)
    setStatusMessage('Uploading package...')

    try {
      // Simulate upload progress (actual progress would need XMLHttpRequest)
      const progressInterval = setInterval(() => {
        setUploadProgress((prev) => {
          if (prev >= 90) {
            clearInterval(progressInterval)
            return prev
          }
          return prev + 10
        })
      }, 100)

      const response = await packagesApi.uploadPackage(repo, selectedFile, currentPath, activeBranch)

      clearInterval(progressInterval)
      setUploadProgress(100)

      // Store package name for navigation after processing
      setPackageName(response.package_name)

      // Check if we got a job_id for background processing
      if (response.job_id) {
        // Large upload - processing in background
        setUploadStatus('processing')
        setStatusMessage('Processing package...')
        setTrackingJobId(response.job_id)
        // SSE handler will update progress and handle completion
      } else {
        // Small upload - already processed
        setUploadStatus('complete')
        setStatusMessage('Package uploaded successfully!')
        showSuccess('Upload Complete', `Package "${response.package_name}" uploaded successfully`)

        // Navigate to package details after a short delay
        setTimeout(() => {
          // Build the full path to the package
          const packagePath = currentPath === '/' || currentPath === ''
            ? response.package_name
            : `${currentPath.replace(/^\//, '')}/${response.package_name}`
          navigate(`/${repo}/${activeBranch}/packages/${packagePath}`)
        }, 1500)
      }
    } catch (error) {
      console.error('Failed to upload package:', error)
      setUploadStatus('error')
      setStatusMessage(error instanceof Error ? error.message : 'Failed to upload package')
      showError('Upload Failed', error instanceof Error ? error.message : 'Failed to upload package')
      setUploadProgress(0)
    }
  }

  function handleCancel() {
    setSelectedFile(null)
    setUploadStatus('idle')
    setUploadProgress(0)
    setProcessingProgress(0)
    setStatusMessage('')
    setTrackingJobId(null)
    setPackageName(null)
  }

  const isUploading = uploadStatus === 'uploading'
  const isProcessing = uploadStatus === 'processing'
  const isComplete = uploadStatus === 'complete'
  const isError = uploadStatus === 'error'
  const isBusy = isUploading || isProcessing

  // Build the back navigation URL
  const backUrl = currentPath === '/' || currentPath === ''
    ? `/${repo}/${activeBranch}/packages`
    : `/${repo}/${activeBranch}/packages/${currentPath.replace(/^\//, '')}`

  // Format the current path for display
  const displayPath = currentPath === '/' || currentPath === '' ? 'root' : currentPath

  return (
    <div className="animate-fade-in">
      {/* Header */}
      <div className="mb-8">
        <button
          onClick={() => navigate(backUrl)}
          className="flex items-center gap-2 text-zinc-400 hover:text-white mb-4 transition-colors"
        >
          <ArrowLeft className="w-4 h-4" />
          Back to Packages
        </button>

        <h1 className="text-4xl font-bold text-white mb-2">Upload Package</h1>
        <p className="text-zinc-400">
          Upload a RaisinDB package file (.rap)
          {currentPath && currentPath !== '/' && (
            <span className="ml-2 inline-flex items-center gap-1 text-primary-400">
              <FolderOpen className="w-4 h-4" />
              to {displayPath}
            </span>
          )}
        </p>
      </div>

      <div className="max-w-2xl mx-auto">
        {/* Upload Area */}
        {!selectedFile ? (
          <GlassCard>
            <div
              onDragOver={handleDragOver}
              onDragLeave={handleDragLeave}
              onDrop={handleDrop}
              onClick={() => fileInputRef.current?.click()}
              className={`
                border-2 border-dashed rounded-xl p-12 text-center cursor-pointer
                transition-all duration-200
                ${
                  isDragging
                    ? 'border-primary-400 bg-primary-500/10'
                    : 'border-white/20 hover:border-primary-400 hover:bg-white/5'
                }
              `}
            >
              <Upload className="w-16 h-16 mx-auto mb-4 text-primary-400" />
              <h3 className="text-xl font-semibold text-white mb-2">
                Drop your package file here
              </h3>
              <p className="text-zinc-400 mb-4">or click to browse</p>
              <p className="text-sm text-zinc-500">Accepts .rap files only</p>

              <input
                ref={fileInputRef}
                type="file"
                accept=".rap"
                onChange={handleFileInputChange}
                className="hidden"
              />
            </div>
          </GlassCard>
        ) : (
          <>
            {/* Selected File Info */}
            <GlassCard className="mb-6">
              <div className="flex items-center gap-4 mb-4">
                <div className="w-12 h-12 rounded-lg bg-primary-500/20 flex items-center justify-center flex-shrink-0">
                  <File className="w-6 h-6 text-primary-400" />
                </div>
                <div className="flex-1 min-w-0">
                  <h3 className="text-lg font-semibold text-white truncate">
                    {selectedFile.name}
                  </h3>
                  <p className="text-sm text-zinc-400">
                    {(selectedFile.size / 1024 / 1024).toFixed(2)} MB
                  </p>
                </div>
                {!isBusy && !isComplete && (
                  <button
                    onClick={handleCancel}
                    className="p-2 text-zinc-400 hover:text-white transition-colors"
                    title="Remove file"
                  >
                    <XCircle className="w-5 h-5" />
                  </button>
                )}
              </div>

              {/* Upload Progress Bar */}
              {isUploading && (
                <div className="mb-4">
                  <div className="flex items-center justify-between mb-2">
                    <span className="text-sm text-zinc-400">Uploading...</span>
                    <span className="text-sm text-white">{uploadProgress}%</span>
                  </div>
                  <div className="w-full bg-white/10 rounded-full h-2 overflow-hidden">
                    <div
                      className="bg-primary-500 h-full transition-all duration-300"
                      style={{ width: `${uploadProgress}%` }}
                    />
                  </div>
                </div>
              )}

              {/* Processing Progress Bar */}
              {isProcessing && (
                <div className="mb-4">
                  <div className="flex items-center justify-between mb-2">
                    <div className="flex items-center gap-2">
                      <Loader2 className="w-4 h-4 text-primary-400 animate-spin" />
                      <span className="text-sm text-zinc-400">Processing package...</span>
                    </div>
                    <span className="text-sm text-white">{processingProgress}%</span>
                  </div>
                  <div className="w-full bg-white/10 rounded-full h-2 overflow-hidden">
                    <div
                      className="bg-primary-500 h-full transition-all duration-300"
                      style={{ width: `${processingProgress}%` }}
                    />
                  </div>
                  {statusMessage && (
                    <p className="text-xs text-zinc-500 mt-1">{statusMessage}</p>
                  )}
                </div>
              )}

              {/* Success Status */}
              {isComplete && (
                <div className="flex items-center gap-2 text-green-400 mb-4">
                  <CheckCircle className="w-5 h-5" />
                  <span className="text-sm font-medium">{statusMessage || 'Upload complete!'}</span>
                </div>
              )}

              {/* Error Status */}
              {isError && (
                <div className="flex items-center gap-2 text-red-400 mb-4">
                  <XCircle className="w-5 h-5" />
                  <span className="text-sm font-medium">{statusMessage || 'Upload failed'}</span>
                </div>
              )}
            </GlassCard>

            {/* Info Notice */}
            <GlassCard className="mb-6">
              <div className="flex items-start gap-3">
                <AlertCircle className="w-5 h-5 text-blue-400 flex-shrink-0 mt-0.5" />
                <div>
                  <h4 className="text-white font-medium mb-2">Before uploading</h4>
                  <ul className="text-sm text-zinc-400 space-y-1 list-disc list-inside">
                    <li>Ensure the package is a valid .rap file</li>
                    <li>Check that all dependencies are available</li>
                    <li>Review package manifest for correct metadata</li>
                    <li>Backup your repository before installing</li>
                  </ul>
                </div>
              </div>
            </GlassCard>

            {/* Action Buttons */}
            <div className="flex gap-4">
              <button
                onClick={handleUpload}
                disabled={isBusy || isComplete}
                className="flex-1 flex items-center justify-center gap-2 px-6 py-3 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
              >
                {isUploading ? (
                  <>
                    <div className="w-5 h-5 border-2 border-white/30 border-t-white rounded-full animate-spin" />
                    Uploading...
                  </>
                ) : isProcessing ? (
                  <>
                    <Loader2 className="w-5 h-5 animate-spin" />
                    Processing...
                  </>
                ) : isComplete ? (
                  <>
                    <CheckCircle className="w-5 h-5" />
                    Complete
                  </>
                ) : isError ? (
                  <>
                    <Upload className="w-5 h-5" />
                    Retry Upload
                  </>
                ) : (
                  <>
                    <Upload className="w-5 h-5" />
                    Upload Package
                  </>
                )}
              </button>

              {!isBusy && !isComplete && (
                <button
                  onClick={() => navigate(backUrl)}
                  className="px-6 py-3 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
                >
                  Cancel
                </button>
              )}
            </div>
          </>
        )}
      </div>

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
