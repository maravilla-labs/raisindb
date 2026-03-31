/**
 * Step Node Component
 *
 * Basic workflow step card.
 * Supports light and dark themes.
 */

import { clsx } from 'clsx';
import { useState, useRef, useEffect, useCallback } from 'react';
import {
  Square,
  Code2,
  Bot,
  AlertCircle,
  Pencil,
  Trash2,
  SquareCheck as CheckSquare,
  Link2,
  UserCheck,
  MessageSquare,
  GitBranch,
  Loader2,
  Check,
  X,
  Clock,
} from 'lucide-react';
import type { FlowStep, InsertPosition } from '../../types';
import { getRefDisplayName, getRefPath } from '../../types';
import { useThemeClasses } from '../../context';

/** Execution status for visualization */
export type NodeExecutionStatus = 'idle' | 'running' | 'completed' | 'failed' | 'waiting';

export interface StepNodeProps {
  /** Step data */
  node: FlowStep;
  /** Parent container ID (null for root) */
  parentId?: string | null;
  /** Next sibling ID (used for drop validation) */
  nextSiblingId?: string | null;
  /** Whether this node is selected (checkbox reflects this) */
  selected?: boolean;
  /** Execution status for visual highlighting during flow execution */
  executionStatus?: NodeExecutionStatus;
  /** Click handler (for selection toggle) */
  onClick?: () => void;
  /** Handler to open function picker */
  onOpenFunctionPicker?: () => void;
  /** Handler to open agent picker */
  onOpenAgentPicker?: () => void;
  /** Handler to unlink/remove function from step */
  onUnlinkFunction?: (nodeId: string) => void;
  /** Handler to unlink/remove agent from step */
  onUnlinkAgent?: (nodeId: string) => void;
  /** Title update handler */
  onUpdateTitle?: (nodeId: string, newTitle: string) => void;
  /** Handler for adding a step relative to this one */
  onAddStep?: (position: InsertPosition) => void;
  /** Drag handlers from useDragAndDrop */
  dragHandlers?: {
    onPointerDown: (e: React.PointerEvent) => void;
  };
  /** Whether the node is disabled for interaction */
  disabled?: boolean;
  /** Custom class name */
  className?: string;
}

const MIN_TITLE_LENGTH = 4;

function validateTitle(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) {
    return 'Title cannot be empty';
  }
  if (trimmed.length < MIN_TITLE_LENGTH) {
    return `Title must be at least ${MIN_TITLE_LENGTH} characters`;
  }
  return null;
}

export function StepNode({
  node,
  parentId,
  nextSiblingId,
  selected = false,
  executionStatus = 'idle',
  onClick,
  onOpenFunctionPicker,
  onOpenAgentPicker,
  onUnlinkFunction,
  onUnlinkAgent,
  onUpdateTitle,
  dragHandlers,
  disabled = false,
  className,
}: StepNodeProps) {
  const themeClasses = useThemeClasses();
  const title = node.properties?.action || 'Untitled Step';
  const isDisabled = node.properties?.disabled;
  const stepType = node.properties?.step_type;
  const isHumanTask = stepType === 'human_task';
  const isChatStep = stepType === 'chat';
  const isAIAgent = stepType === 'ai_agent' || !!node.properties?.agent_ref;
  const isIsolatedBranch = !!node.properties?.isolated_branch;
  const functionRef = node.properties?.function_ref;
  const agentRef = node.properties?.agent_ref;
  const chatConfig = node.properties?.chat_config;
  const hasFunction = !!functionRef;
  const hasAgent = !!agentRef;
  const hasChatAgent = !!(chatConfig?.agent_ref);

  // Inline editing state
  const [isEditing, setIsEditing] = useState(false);
  const [editValue, setEditValue] = useState(title);
  const [cursorOffset, setCursorOffset] = useState(0);
  const [validationError, setValidationError] = useState<string | null>(null);
  const editRef = useRef<HTMLSpanElement>(null);

  // Track if we're waiting to distinguish click from double-click
  const clickTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingClickRef = useRef<{ x: number; y: number } | null>(null);

  // Position cursor after entering edit mode
  useEffect(() => {
    if (isEditing && editRef.current) {
      // Set content and focus
      editRef.current.textContent = editValue;
      editRef.current.focus();

      // Position cursor at exact offset
      const selection = window.getSelection();
      const textNode = editRef.current.firstChild;
      if (selection && textNode) {
        const maxOffset = textNode.textContent?.length ?? 0;
        selection.collapse(textNode, Math.min(cursorOffset, maxOffset));
      }
    }
  }, [isEditing, cursorOffset, editValue]);

  // Sync editValue when title changes externally
  useEffect(() => {
    if (!isEditing) {
      setEditValue(title);
    }
  }, [title, isEditing]);

  const handleCheckboxClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    // Toggle selection - checkbox reflects selected state
    onClick?.();
  };

  const handleTitleClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();

      // Store click position for potential delayed execution
      pendingClickRef.current = { x: e.clientX, y: e.clientY };

      // Clear any existing timeout
      if (clickTimeoutRef.current) {
        clearTimeout(clickTimeoutRef.current);
      }

      // Delay to distinguish from double-click (200ms)
      clickTimeoutRef.current = setTimeout(() => {
        const pending = pendingClickRef.current;
        if (pending) {
          // Calculate cursor offset from click position
          const range = document.caretRangeFromPoint(pending.x, pending.y);
          const offset = range?.startOffset ?? title.length;

          setEditValue(title);
          setCursorOffset(offset);
          setValidationError(null);
          setIsEditing(true);
        }
        pendingClickRef.current = null;
      }, 200);
    },
    [title]
  );

  const handleTitleDoubleClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();

      // Cancel pending single-click
      if (clickTimeoutRef.current) {
        clearTimeout(clickTimeoutRef.current);
        clickTimeoutRef.current = null;
      }
      pendingClickRef.current = null;

      // Select all text on double-click (standard behavior)
      const selection = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(e.currentTarget);
      selection?.removeAllRanges();
      selection?.addRange(range);
    },
    []
  );

  const handleBlur = useCallback(() => {
    if (!editRef.current) return;

    const newValue = editRef.current.textContent || '';
    const error = validateTitle(newValue);

    if (error) {
      // Revert to original on validation error
      setValidationError(null);
      setIsEditing(false);
    } else {
      // Save valid value
      if (newValue.trim() !== title) {
        onUpdateTitle?.(node.id, newValue.trim());
      }
      setValidationError(null);
      setIsEditing(false);
    }
  }, [title, node.id, onUpdateTitle]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        const newValue = editRef.current?.textContent || '';
        const error = validateTitle(newValue);

        if (error) {
          setValidationError(error);
          // Keep focus, don't exit edit mode
        } else {
          if (newValue.trim() !== title) {
            onUpdateTitle?.(node.id, newValue.trim());
          }
          setValidationError(null);
          setIsEditing(false);
        }
      } else if (e.key === 'Escape') {
        // Cancel edit
        setValidationError(null);
        setIsEditing(false);
      }
    },
    [title, node.id, onUpdateTitle]
  );

  const handleInput = useCallback(() => {
    // Clear validation error on input
    if (validationError) {
      setValidationError(null);
    }
  }, [validationError]);

  const handleLinkClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (isAIAgent) {
      onOpenAgentPicker?.();
    } else {
      onOpenFunctionPicker?.();
    }
  };

  const handleUnlinkClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (isAIAgent) {
      onUnlinkAgent?.(node.id);
    } else {
      onUnlinkFunction?.(node.id);
    }
  };

  // Cleanup timeout on unmount
  useEffect(() => {
    return () => {
      if (clickTimeoutRef.current) {
        clearTimeout(clickTimeoutRef.current);
      }
    };
  }, []);

  // Execution state styling
  const executionStyles = {
    running: 'ring-2 ring-blue-500 ring-offset-2 ring-offset-transparent animate-pulse',
    completed: 'ring-2 ring-green-500 ring-offset-2 ring-offset-transparent',
    failed: 'ring-2 ring-red-500 ring-offset-2 ring-offset-transparent',
    waiting: 'ring-2 ring-yellow-500 ring-offset-2 ring-offset-transparent animate-pulse',
    idle: '',
  };

  return (
    <div
      data-flow-node-id={node.id}
      data-flow-node-container="false"
      data-flow-node-parent-id={parentId ?? ''}
      data-flow-node-next-id={nextSiblingId ?? ''}
      className={clsx(
        'transition relative duration-400',
        'rounded-lg p-4',
        themeClasses.stepShadow,
        'select-none flex flex-col items-center',
        'w-[300px]',
        // Selection state (only when not executing)
        executionStatus === 'idle' && selected && clsx('outline-2 outline', themeClasses.stepOutline, themeClasses.stepOutlineHover),
        executionStatus === 'idle' && !selected && 'hover:outline-2 hover:outline hover:outline-blue-200/50',
        // Execution state styling (takes precedence over selection)
        executionStyles[executionStatus],
        // Disabled/enabled state
        isDisabled ? themeClasses.stepBgDisabled : themeClasses.stepBg,
        isDisabled ? 'text-gray-400' : themeClasses.stepText,
        // Interaction disabled
        disabled && 'pointer-events-none opacity-50',
        className
      )}
      {...dragHandlers}
    >
      {/* Checkbox - click toggles selection, vertically centered */}
      <button
        onClick={handleCheckboxClick}
        className={clsx(
          'absolute top-3 left-3 cursor-pointer',
          themeClasses.isDark ? 'text-gray-500 hover:text-gray-300' : 'text-gray-400 hover:text-gray-600'
        )}
        title={selected ? 'Deselect step' : 'Select step'}
      >
        {selected ? (
          <CheckSquare className="w-5 h-5 text-blue-500" />
        ) : (
          <Square className="w-5 h-5" />
        )}
      </button>

      {/* Isolated branch indicator */}
      {isIsolatedBranch && (
        <div
          className={clsx(
            'absolute top-3 left-10',
            themeClasses.isDark ? 'text-cyan-400' : 'text-cyan-600'
          )}
          title="Executes in isolated branch"
        >
          <GitBranch className="w-4 h-4" />
        </div>
      )}

      {/* Execution status indicator (takes precedence when executing) */}
      {executionStatus === 'running' && (
        <div className="absolute top-3 right-3" title="Running">
          <Loader2 className="w-5 h-5 text-blue-500 animate-spin" />
        </div>
      )}
      {executionStatus === 'completed' && (
        <div className="absolute top-3 right-3" title="Completed">
          <Check className="w-5 h-5 text-green-500" />
        </div>
      )}
      {executionStatus === 'failed' && (
        <div className="absolute top-3 right-3" title="Failed">
          <X className="w-5 h-5 text-red-500" />
        </div>
      )}
      {executionStatus === 'waiting' && (
        <div className="absolute top-3 right-3" title="Waiting">
          <Clock className="w-5 h-5 text-yellow-500 animate-pulse" />
        </div>
      )}

      {/* Error indicator - incomplete step configuration (only shown when idle) */}
      {executionStatus === 'idle' && (
        isHumanTask ? !node.properties?.assignee :
        isChatStep ? !hasChatAgent :
        !hasFunction && !hasAgent
      ) && (
        <div
          className="absolute top-3 right-3"
          title={
            isHumanTask
              ? "Step incomplete: No assignee"
              : isChatStep
                ? "Step incomplete: No chat agent selected"
                : isAIAgent
                  ? "Step incomplete: No agent selected"
                  : "Step incomplete: No function selected"
          }
        >
          <AlertCircle className="w-5 h-5 text-red-500" />
        </div>
      )}

      {/* Title - inline editable */}
      {isEditing ? (
        <div className="flex flex-col items-center">
          <span
            ref={editRef}
            contentEditable
            suppressContentEditableWarning
            className={clsx(
              'text-lg font-medium mb-2 outline-none',
              'min-w-[50px] inline-block text-center',
              'px-1 rounded',
              themeClasses.stepText,
              validationError
                ? 'ring-2 ring-red-400 text-red-600'
                : 'ring-2 ring-blue-300',
              themeClasses.isDark && !validationError && 'ring-blue-500'
            )}
            onBlur={handleBlur}
            onKeyDown={handleKeyDown}
            onInput={handleInput}
          />
          {validationError && (
            <span className="text-xs text-red-500 -mt-1 mb-1">
              {validationError}
            </span>
          )}
        </div>
      ) : (
        <h1
          className={clsx('text-lg font-medium mb-2 hover:cursor-text px-1', themeClasses.stepText)}
          onClick={handleTitleClick}
          onDoubleClick={handleTitleDoubleClick}
        >
          {title}
        </h1>
      )}

      {/* Function/Agent/Human Task/Chat info display */}
      {isHumanTask ? (
        // Human Task display
        <div className="flex flex-col items-center w-full mt-1">
          <div className={clsx('flex items-center gap-1.5 text-sm', 'text-amber-800 dark:text-amber-300')}>
            <UserCheck className={clsx('w-4 h-4 flex-shrink-0', 'text-amber-800 dark:text-amber-300')} />
            <span className="font-medium capitalize">
              {node.properties?.task_type || 'Task'}
            </span>
          </div>
          {node.properties?.assignee ? (
            <span className={clsx('text-xs mt-0.5', 'text-amber-700 dark:text-amber-400')}>
              → {node.properties.assignee}
            </span>
          ) : (
            <span className={clsx('text-xs mt-0.5 italic', themeClasses.stepTextFaint)}>
              No assignee
            </span>
          )}
        </div>
      ) : isChatStep ? (
        // Chat Step display
        <div className="flex flex-col items-center w-full mt-1">
          <div className={clsx('flex items-center gap-1.5 text-sm', 'text-cyan-700 dark:text-cyan-300')}>
            <MessageSquare className={clsx('w-4 h-4 flex-shrink-0', 'text-cyan-700 dark:text-cyan-300')} />
            <span className="font-medium">Chat Session</span>
          </div>
          {chatConfig?.agent_ref ? (
            <>
              <span className={clsx('text-xs mt-0.5', 'text-cyan-600 dark:text-cyan-400')}>
                {getRefDisplayName(chatConfig.agent_ref)}
              </span>
              {chatConfig.max_turns && (
                <span className={clsx('text-xs', themeClasses.stepTextFaint)}>
                  Max {chatConfig.max_turns} turns
                </span>
              )}
              {chatConfig.handoff_targets && chatConfig.handoff_targets.length > 0 && (
                <span className={clsx('text-xs', themeClasses.stepTextFaint)}>
                  {chatConfig.handoff_targets.length} handoff target{chatConfig.handoff_targets.length > 1 ? 's' : ''}
                </span>
              )}
            </>
          ) : (
            <span className={clsx('text-xs mt-0.5 italic', themeClasses.stepTextFaint)}>
              No chat agent configured
            </span>
          )}
        </div>
      ) : isAIAgent ? (
        // AI Agent display
        hasAgent && agentRef ? (
          <div className="flex flex-col items-center w-full mt-1">
            <div className={clsx('flex items-center gap-1.5 text-sm', 'text-purple-700 dark:text-purple-300')}>
              <Bot className={clsx('w-4 h-4 flex-shrink-0', 'text-purple-700 dark:text-purple-300')} />
              <span className="font-medium">{getRefDisplayName(agentRef)}</span>
              {/* Edit button to change agent */}
              <button
                onClick={handleLinkClick}
                className={clsx('p-0.5 transition-colors', themeClasses.stepTextFaint, 'hover:text-purple-500')}
                title="Change agent"
              >
                <Pencil className="w-3.5 h-3.5" />
              </button>
              {/* Delete button to remove agent */}
              <button
                onClick={handleUnlinkClick}
                className={clsx('p-0.5 transition-colors', themeClasses.stepTextFaint, 'hover:text-red-500')}
                title="Remove agent"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
            {/* Path */}
            <span
              className={clsx('text-xs truncate max-w-full mt-0.5', themeClasses.stepTextFaint)}
              title={getRefPath(agentRef)}
            >
              {getRefPath(agentRef)}
            </span>
            {/* Workspace */}
            <span className={clsx('text-xs mt-0.5', themeClasses.isDark ? 'text-gray-500' : 'text-gray-300')}>
              @ {agentRef['raisin:workspace']}
            </span>
          </div>
        ) : (
          // No agent selected - show message and add button
          <div className="flex flex-col items-center mt-1">
            <span className={clsx('text-sm', themeClasses.stepTextFaint)}>No agent selected</span>
            <button
              onClick={handleLinkClick}
              className="flex items-center gap-1 text-sm text-purple-500 hover:text-purple-700 mt-1 transition-colors"
            >
              <Link2 className="w-4 h-4" />
              <span>Add agent</span>
            </button>
          </div>
        )
      ) : (
        // Regular function step display
        hasFunction && functionRef ? (
          <div className="flex flex-col items-center w-full mt-1">
            <div className={clsx('flex items-center gap-1.5 text-sm', themeClasses.stepTextMuted)}>
              <Code2 className={clsx('w-4 h-4 flex-shrink-0', themeClasses.stepTextMuted)} />
              <span className="font-medium">{getRefDisplayName(functionRef)}</span>
              {/* Edit button to change function */}
              <button
                onClick={handleLinkClick}
                className={clsx('p-0.5 transition-colors', themeClasses.stepTextFaint, 'hover:text-blue-500')}
                title="Change function"
              >
                <Pencil className="w-3.5 h-3.5" />
              </button>
              {/* Delete button to remove function */}
              <button
                onClick={handleUnlinkClick}
                className={clsx('p-0.5 transition-colors', themeClasses.stepTextFaint, 'hover:text-red-500')}
                title="Remove function"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
            {/* Path */}
            <span
              className={clsx('text-xs truncate max-w-full mt-0.5', themeClasses.stepTextFaint)}
              title={getRefPath(functionRef)}
            >
              {getRefPath(functionRef)}
            </span>
            {/* Workspace */}
            <span className={clsx('text-xs mt-0.5', themeClasses.isDark ? 'text-gray-500' : 'text-gray-300')}>
              @ {functionRef['raisin:workspace']}
            </span>
          </div>
        ) : (
          // No function selected - show message and add button
          <div className="flex flex-col items-center mt-1">
            <span className={clsx('text-sm', themeClasses.stepTextFaint)}>No function selected</span>
            <button
              onClick={handleLinkClick}
              className="flex items-center gap-1 text-sm text-blue-500 hover:text-blue-700 mt-1 transition-colors"
            >
              <Link2 className="w-4 h-4" />
              <span>Add function</span>
            </button>
          </div>
        )
      )}
    </div>
  );
}
