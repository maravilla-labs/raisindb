/**
 * Flow Toolbar Component
 *
 * Toolbar with actions matching the design specification.
 */

import { clsx } from 'clsx';
import {
  Save,
  Undo2,
  Redo2,
  Trash2,
  Hand,
  MousePointer2,
  ZoomIn,
  ZoomOut,
  PanelLeft,
  PanelLeftClose,
  PanelRight,
  PanelRightClose,
  Play,
  Square,
  FlaskConical,
  Loader2,
  CheckCircle,
  XCircle,
  Sun,
  Moon,
} from 'lucide-react';
import type { FlowTheme } from '../context';
import type { FlowExecutionStatus } from '../types/flow';

export interface FlowToolbarProps {
  /** Whether sidebar/left panel is visible */
  sidebarVisible?: boolean;
  /** Handler for toggling sidebar/left panel */
  onToggleSidebar?: () => void;
  /** Handler for save action */
  onSave?: () => void;
  /** Handler for undo */
  onUndo?: () => void;
  /** Handler for redo */
  onRedo?: () => void;
  /** Whether undo is available */
  canUndo?: boolean;
  /** Whether redo is available */
  canRedo?: boolean;
  /** Handler for delete selected */
  onDelete?: () => void;
  /** Whether delete is available */
  canDelete?: boolean;
  /** Current tool mode */
  toolMode?: 'select' | 'pan';
  /** Handler for tool mode change */
  onToolModeChange?: (mode: 'select' | 'pan') => void;
  /** Handler for zoom in */
  onZoomIn?: () => void;
  /** Handler for zoom out */
  onZoomOut?: () => void;
  /** Current zoom percentage */
  currentZoom?: number;
  /** Whether properties panel is visible */
  propertiesVisible?: boolean;
  /** Handler for toggling properties panel */
  onToggleProperties?: () => void;
  /** Handler for run action */
  onRun?: () => void;
  /** Handler for test run action */
  onTestRun?: () => void;
  /** Handler for stop action */
  onStop?: () => void;
  /** Current execution status */
  executionStatus?: FlowExecutionStatus;
  /** Current canvas theme */
  canvasTheme?: FlowTheme;
  /** Handler for toggling canvas theme */
  onToggleCanvasTheme?: () => void;
  /** Custom class name */
  className?: string;
}

function ToolbarDivider() {
  return (
    <div className="h-6 w-px bg-white/20 mx-1" />
  );
}

export function FlowToolbar({
  sidebarVisible = true,
  onToggleSidebar,
  onSave,
  onUndo,
  onRedo,
  canUndo = false,
  canRedo = false,
  onDelete,
  canDelete = false,
  toolMode = 'select',
  onToolModeChange,
  onZoomIn,
  onZoomOut,
  currentZoom = 100,
  propertiesVisible = true,
  onToggleProperties,
  onRun,
  onTestRun,
  onStop,
  executionStatus = 'idle',
  canvasTheme = 'dark',
  onToggleCanvasTheme,
  className,
}: FlowToolbarProps) {
  const isExecuting = executionStatus === 'running' || executionStatus === 'waiting';
  return (
    <div
      className={clsx(
        'flex items-center gap-0.5 px-2 py-1.5',
        'bg-black/30 backdrop-blur-md border-b border-white/10',
        className
      )}
    >
      {/* Sidebar toggle */}
      {onToggleSidebar && (
        <button
          onClick={onToggleSidebar}
          className={clsx(
            'p-2 rounded transition-colors',
            sidebarVisible
              ? 'text-white bg-white/10'
              : 'text-gray-400 hover:text-white hover:bg-white/10'
          )}
          title={sidebarVisible ? 'Hide sidebar' : 'Show sidebar'}
        >
          {sidebarVisible ? (
            <PanelLeftClose className="w-5 h-5" />
          ) : (
            <PanelLeft className="w-5 h-5" />
          )}
        </button>
      )}

      {/* Save button */}
      <button
        onClick={onSave}
        className="p-2 rounded text-gray-400 hover:text-white hover:bg-white/10 transition-colors"
        title="Save"
      >
        <Save className="w-5 h-5" />
      </button>

      <ToolbarDivider />

      {/* Undo */}
      <button
        onClick={onUndo}
        disabled={!canUndo}
        className={clsx(
          'p-2 rounded transition-colors',
          canUndo
            ? 'text-gray-400 hover:text-white hover:bg-white/10'
            : 'text-gray-600 cursor-not-allowed'
        )}
        title="Undo (Ctrl+Z)"
      >
        <Undo2 className="w-5 h-5" />
      </button>

      {/* Redo */}
      <button
        onClick={onRedo}
        disabled={!canRedo}
        className={clsx(
          'p-2 rounded transition-colors',
          canRedo
            ? 'text-gray-400 hover:text-white hover:bg-white/10'
            : 'text-gray-600 cursor-not-allowed'
        )}
        title="Redo (Ctrl+Y)"
      >
        <Redo2 className="w-5 h-5" />
      </button>

      <ToolbarDivider />

      {/* Delete */}
      <button
        onClick={onDelete}
        disabled={!canDelete}
        className={clsx(
          'p-2 rounded transition-colors',
          canDelete
            ? 'text-gray-400 hover:text-red-400 hover:bg-red-500/10'
            : 'text-gray-600 cursor-not-allowed'
        )}
        title="Delete selected"
      >
        <Trash2 className="w-5 h-5" />
      </button>

      <ToolbarDivider />

      {/* Select tool */}
      <button
        onClick={() => onToolModeChange?.('select')}
        className={clsx(
          'p-2 rounded transition-colors',
          toolMode === 'select'
            ? 'text-white bg-white/10'
            : 'text-gray-400 hover:text-white hover:bg-white/10'
        )}
        title="Select tool"
      >
        <MousePointer2 className="w-5 h-5" />
      </button>

      {/* Pan tool */}
      <button
        onClick={() => onToolModeChange?.('pan')}
        className={clsx(
          'p-2 rounded transition-colors',
          toolMode === 'pan'
            ? 'text-white bg-white/10'
            : 'text-gray-400 hover:text-white hover:bg-white/10'
        )}
        title="Pan tool"
      >
        <Hand className="w-5 h-5" />
      </button>

      {/* Spacer */}
      <div className="flex-1" />

      {/* Properties toggle */}
      {onToggleProperties && (
        <button
          onClick={onToggleProperties}
          className={clsx(
            'p-2 rounded transition-colors',
            propertiesVisible
              ? 'text-white bg-white/10'
              : 'text-gray-400 hover:text-white hover:bg-white/10'
          )}
          title={propertiesVisible ? 'Hide properties' : 'Show properties'}
        >
          {propertiesVisible ? (
            <PanelRightClose className="w-5 h-5" />
          ) : (
            <PanelRight className="w-5 h-5" />
          )}
        </button>
      )}

      {/* Canvas theme toggle */}
      {onToggleCanvasTheme && (
        <>
          <ToolbarDivider />
          <button
            onClick={onToggleCanvasTheme}
            className={clsx(
              'p-2 rounded transition-colors',
              'text-gray-400 hover:text-white hover:bg-white/10'
            )}
            title={canvasTheme === 'dark' ? 'Switch to light canvas' : 'Switch to dark canvas'}
          >
            {canvasTheme === 'dark' ? (
              <Sun className="w-5 h-5" />
            ) : (
              <Moon className="w-5 h-5" />
            )}
          </button>
        </>
      )}

      <ToolbarDivider />

      {/* Zoom controls */}
      <button
        onClick={onZoomOut}
        className="p-2 rounded text-gray-400 hover:text-white hover:bg-white/10 transition-colors"
        title="Zoom out"
      >
        <ZoomOut className="w-5 h-5" />
      </button>

      <span className="text-sm text-gray-400 min-w-[55px] text-center font-medium">
        {currentZoom}%
      </span>

      <button
        onClick={onZoomIn}
        className="p-2 rounded text-gray-400 hover:text-white hover:bg-white/10 transition-colors"
        title="Zoom in"
      >
        <ZoomIn className="w-5 h-5" />
      </button>

      {/* Execution controls */}
      {(onRun || onTestRun) && (
        <>
          <ToolbarDivider />

          {/* Execution status indicator */}
          {executionStatus === 'running' && (
            <div className="flex items-center gap-1.5 px-2 text-blue-400">
              <Loader2 className="w-4 h-4 animate-spin" />
              <span className="text-sm">Running...</span>
            </div>
          )}
          {executionStatus === 'waiting' && (
            <div className="flex items-center gap-1.5 px-2 text-yellow-400">
              <Loader2 className="w-4 h-4 animate-pulse" />
              <span className="text-sm">Waiting...</span>
            </div>
          )}
          {executionStatus === 'completed' && (
            <div className="flex items-center gap-1.5 px-2 text-green-400">
              <CheckCircle className="w-4 h-4" />
              <span className="text-sm">Completed</span>
            </div>
          )}
          {executionStatus === 'failed' && (
            <div className="flex items-center gap-1.5 px-2 text-red-400">
              <XCircle className="w-4 h-4" />
              <span className="text-sm">Failed</span>
            </div>
          )}

          {/* Stop button (shown when executing) */}
          {isExecuting && onStop && (
            <button
              onClick={onStop}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-red-600 hover:bg-red-700 text-white transition-colors"
              title="Stop execution"
            >
              <Square className="w-4 h-4" />
              <span className="text-sm font-medium">Stop</span>
            </button>
          )}

          {/* Test Run button */}
          {!isExecuting && onTestRun && (
            <button
              onClick={onTestRun}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-purple-600 hover:bg-purple-700 text-white transition-colors"
              title="Test run with mocking"
            >
              <FlaskConical className="w-4 h-4" />
              <span className="text-sm font-medium">Test</span>
            </button>
          )}

          {/* Run button */}
          {!isExecuting && onRun && (
            <button
              onClick={onRun}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-green-600 hover:bg-green-700 text-white transition-colors"
              title="Run flow"
            >
              <Play className="w-4 h-4" />
              <span className="text-sm font-medium">Run</span>
            </button>
          )}
        </>
      )}
    </div>
  );
}
