/**
 * Container Node Component
 *
 * AND/OR/Parallel/AI container with nested children.
 * Supports light and dark themes.
 */

import { useState } from 'react';
import { clsx } from 'clsx';
import { Sparkles, GitBranch, Layers, ChevronUp, ChevronDown } from 'lucide-react';
import type { FlowContainer, FlowNode, ContainerType } from '../../types';
import { ContainerTypeIcon } from '../connections/ContainerTypeIcon';
import { useThemeClasses } from '../../context';

export interface ContainerNodeProps {
  /** Container data */
  node: FlowContainer;
  /** Parent container ID (null for root) */
  parentId?: string | null;
  /** Next sibling ID */
  nextSiblingId?: string | null;
  /** Whether this node is selected */
  selected?: boolean;
  /** Click handler */
  onClick?: () => void;
  /** Handler to add a child step */
  onAddChild?: () => void;
  /** Handler to delete this container */
  onDelete?: () => void;
  /** Drag handlers from useDragAndDrop */
  dragHandlers?: {
    onPointerDown: (e: React.PointerEvent) => void;
  };
  /** Render function for each child */
  renderChild?: (child: FlowNode, index: number, siblings: FlowNode[]) => React.ReactNode;
  /** Whether a drop is targeting this container */
  highlightDropTarget?: boolean;
  /** Whether a drag is currently active (enables drop overlay) */
  isDragging?: boolean;
  /** Whether the node is disabled for interaction */
  disabled?: boolean;
  /** Custom class name */
  className?: string;
}

const CONTAINER_ICONS: Record<ContainerType, typeof Sparkles> = {
  ai_sequence: Sparkles,
  and: GitBranch,
  or: GitBranch,
  parallel: Layers,
};

export function ContainerNode({
  node,
  parentId,
  nextSiblingId,
  selected = false,
  onClick,
  onAddChild,
  dragHandlers,
  renderChild,
  highlightDropTarget = false,
  disabled = false,
  className,
}: ContainerNodeProps) {
  const [collapsed, setCollapsed] = useState(false);
  const themeClasses = useThemeClasses();
  const Icon = CONTAINER_ICONS[node.container_type];
  const hasChildren = node.children.length > 0;
  const childCount = node.children.length;

  // Theme-aware colors - use same light blues but with transparency for dark mode
  const containerBg = themeClasses.isDark ? 'bg-blue-50/10' : 'bg-blue-50';
  const containerBorder = themeClasses.isDark ? 'border-blue-200/30' : 'border-blue-200';
  const headerBg = themeClasses.isDark ? 'bg-blue-100/15' : 'bg-blue-100';
  const headerHover = themeClasses.isDark ? 'hover:bg-blue-200/20' : 'hover:bg-blue-200';
  const iconColor = themeClasses.isDark ? 'text-blue-300' : 'text-blue-700';
  const textColor = themeClasses.isDark ? 'text-blue-100' : 'text-blue-900';
  const lineBg = themeClasses.isDark ? 'bg-blue-200/40' : 'bg-blue-200';
  const dropHighlight = themeClasses.isDark ? 'bg-blue-100/20' : 'bg-blue-100';
  const buttonHover = themeClasses.isDark ? 'hover:bg-blue-100/20' : 'hover:bg-blue-300';

  return (
    <div
      data-flow-node-id={node.id}
      data-flow-node-container="true"
      data-flow-node-parent-id={parentId ?? ''}
      data-flow-node-next-id={nextSiblingId ?? ''}
      className={clsx(
        'rounded-lg overflow-hidden flex flex-col items-center min-w-96 border',
        containerBg,
        containerBorder,
        // Selection state
        selected && 'outline-2 outline outline-blue-500',
        // Drop target highlight
        highlightDropTarget && dropHighlight,
        // Interaction disabled
        disabled && 'pointer-events-none opacity-50',
        className
      )}
      {...dragHandlers}
    >
      {/* Header */}
      <div
        className={clsx(
          'flex items-center space-x-4 cursor-pointer flex-1 w-full p-1',
          headerBg,
          headerHover,
          textColor
        )}
        onClick={(e) => {
          e.stopPropagation();
          onClick?.();
        }}
      >
        <Icon className={clsx('w-7 h-7', iconColor)} />
        <div className="flex text-center w-full items-center justify-center">
          ID: {node.id} {childCount}
        </div>
        <button
          onClick={(e) => {
            e.stopPropagation();
            setCollapsed(!collapsed);
          }}
          className={clsx('p-1 rounded', buttonHover)}
        >
          {collapsed ? <ChevronDown className={clsx('w-7 h-7', iconColor)} /> : <ChevronUp className={clsx('w-7 h-7', iconColor)} />}
        </button>
      </div>

      {collapsed ? (
        <div className={clsx('p-3', textColor)}>
          {childCount} Steps
        </div>
      ) : (
        <>
          {/* Vertical Line Before the Children */}
          <div className="relative">
            <div className={clsx('w-[1px] h-10', lineBg)} data-wf-vertical-line="true" />
            <ContainerTypeIcon
              containerType={node.container_type}
              childCount={childCount}
            />
          </div>

          {/* Horizontal Line (dynamically adjusted width) */}
          {childCount > 1 && (
            <div
              className={clsx('h-[1px]', lineBg)}
              data-wf-horizontal-line="true"
              style={{ width: 'calc(100% - 320px)', margin: '0 auto' }}
            />
          )}

          {/* Container for Children with Connection Lines */}
          <div className="flex gap-4 relative px-[10px]">
            {!hasChildren ? (
              <div
                className={clsx(
                  'justify-center border-dashed border-2 border-transparent transition h-[100px] relative duration-400 rounded-lg p-4 select-none flex flex-col items-center w-[300px]',
                  highlightDropTarget ? dropHighlight : containerBg,
                  textColor
                )}
                data-flow-drop-zone={node.id}
                onClick={(e) => {
                  e.stopPropagation();
                  onAddChild?.();
                }}
              >
                <div className="text-red-500 text-2xl">⚠</div>
                <div>No Workflow Steps added</div>
              </div>
            ) : (
              node.children.map((child, index) => (
                <div
                  key={child.id}
                  className="relative flex flex-col items-center"
                  data-wf-child-container="true"
                >
                  {/* Connecting Vertical Line from Child to Horizontal Line */}
                  {childCount > 1 && (
                    <div className={clsx('w-[1px] min-h-10', lineBg)} data-wf-vertical-line="true" />
                  )}

                  <div
                    className="absolute w-full min-h-10 bg-transparent"
                    data-wf-drop-indicator={child.id}
                  />

                  {/* Child Workflow Items */}
                  {renderChild?.(child, index, node.children)}

                  {/* Bottom vertical connector */}
                  {childCount > 1 && (
                    <div className="relative flex w-full h-full items-center justify-center">
                      <div
                        className={clsx('w-[1px] h-full min-h-10', lineBg)}
                        data-wf-vertical-line="true"
                      />
                      <div
                        className="absolute w-full h-full min-h-10 bg-transparent bottom-0"
                        data-wf-drop-indicator={child.id}
                      />
                    </div>
                  )}
                </div>
              ))
            )}
          </div>

          {/* Horizontal Line After the Children */}
          {childCount > 1 && (
            <div
              className={clsx('h-[1px]', lineBg)}
              style={{ width: 'calc(100% - 320px)', margin: '0 auto' }}
              data-wf-horizontal-line="true"
            />
          )}

          {/* Vertical Line After the Horizontal Line */}
          <div className={clsx('w-[1px] h-10', lineBg)} data-wf-vertical-line="true" />
        </>
      )}
    </div>
  );
}
