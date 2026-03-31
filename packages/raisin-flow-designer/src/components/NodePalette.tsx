/**
 * Node Palette Component
 *
 * Floating vertical bar with draggable step and container types.
 * Uses glass morphism background. Draggable to reposition.
 * Double-click grip to reset to default position.
 */

import { useState, useCallback, useRef, useEffect } from 'react';
import { clsx } from 'clsx';
import { Box, Bot, GitBranch, Layers, Sparkles, GripHorizontal, UserCheck, MessageSquare } from 'lucide-react';
import type { StepType } from '../types';
import { useThemeClasses } from '../context';

// Default position (left side, vertically centered)
const DEFAULT_POSITION = { x: 16, y: -1 }; // y: -1 means "center vertically"

export interface NodePaletteProps {
  /** Handler when drag starts from palette */
  onDragStart?: (type: StepType) => void;
  /** Handler when drag ends */
  onDragEnd?: () => void;
  /** Whether palette is disabled */
  disabled?: boolean;
  /** Custom class name */
  className?: string;
}

interface PaletteItemConfig {
  type: StepType;
  icon: typeof Box;
  label: string;
  description: string;
  lightColor: string;
  darkColor: string;
}

const PALETTE_ITEMS: PaletteItemConfig[] = [
  {
    type: 'step',
    icon: Box,
    label: 'Step',
    description: 'Basic workflow step',
    lightColor: 'bg-white hover:bg-gray-50 text-gray-700 border-gray-200',
    darkColor: 'bg-gray-700 hover:bg-gray-600 text-gray-200 border-gray-600',
  },
  {
    type: 'ai_agent',
    icon: Bot,
    label: 'AI Agent',
    description: 'AI agent step',
    lightColor: 'bg-purple-50 hover:bg-purple-100 text-purple-700 border-purple-200',
    darkColor: 'bg-purple-900/50 hover:bg-purple-800/50 text-purple-300 border-purple-700/50',
  },
  {
    type: 'human_task',
    icon: UserCheck,
    label: 'Task',
    description: 'Human approval or input',
    lightColor: 'bg-amber-50 hover:bg-amber-100 text-amber-700 border-amber-200',
    darkColor: 'bg-amber-900/50 hover:bg-amber-800/50 text-amber-300 border-amber-700/50',
  },
  {
    type: 'chat',
    icon: MessageSquare,
    label: 'Chat',
    description: 'Multi-turn chat session',
    lightColor: 'bg-cyan-50 hover:bg-cyan-100 text-cyan-700 border-cyan-200',
    darkColor: 'bg-cyan-900/50 hover:bg-cyan-800/50 text-cyan-300 border-cyan-700/50',
  },
  {
    type: 'and',
    icon: GitBranch,
    label: 'AND',
    description: 'All children must pass',
    lightColor: 'bg-green-50 hover:bg-green-100 text-green-700 border-green-200',
    darkColor: 'bg-green-900/50 hover:bg-green-800/50 text-green-300 border-green-700/50',
  },
  {
    type: 'or',
    icon: GitBranch,
    label: 'OR',
    description: 'Any child can pass',
    lightColor: 'bg-orange-50 hover:bg-orange-100 text-orange-700 border-orange-200',
    darkColor: 'bg-orange-900/50 hover:bg-orange-800/50 text-orange-300 border-orange-700/50',
  },
  {
    type: 'parallel',
    icon: Layers,
    label: 'Parallel',
    description: 'Execute children concurrently',
    lightColor: 'bg-blue-50 hover:bg-blue-100 text-blue-700 border-blue-200',
    darkColor: 'bg-blue-900/50 hover:bg-blue-800/50 text-blue-300 border-blue-700/50',
  },
  {
    type: 'ai_sequence',
    icon: Sparkles,
    label: 'AI',
    description: 'AI-orchestrated execution',
    lightColor: 'bg-purple-50 hover:bg-purple-100 text-purple-700 border-purple-200',
    darkColor: 'bg-purple-900/50 hover:bg-purple-800/50 text-purple-300 border-purple-700/50',
  },
];

interface PaletteItemProps {
  config: PaletteItemConfig;
  isDark: boolean;
  onDragStart?: (type: StepType) => void;
  onDragEnd?: () => void;
  disabled?: boolean;
}

function PaletteItem({ config, isDark, onDragStart, onDragEnd, disabled }: PaletteItemProps) {
  const [isDragging, setIsDragging] = useState(false);
  const Icon = config.icon;

  const handleDragStart = useCallback(
    (e: React.DragEvent) => {
      if (disabled) {
        e.preventDefault();
        return;
      }
      e.dataTransfer.setData('application/x-flow-node-type', config.type);
      e.dataTransfer.effectAllowed = 'copy';
      setIsDragging(true);
      onDragStart?.(config.type);
    },
    [config.type, disabled, onDragStart]
  );

  const handleDragEnd = useCallback(() => {
    setIsDragging(false);
    onDragEnd?.();
  }, [onDragEnd]);

  return (
    <div
      title={`${config.label}: ${config.description}`}
      draggable={!disabled}
      onDragStart={handleDragStart}
      onDragEnd={handleDragEnd}
      className={clsx(
        'w-9 h-9 rounded flex flex-col items-center justify-center',
        'cursor-grab active:cursor-grabbing',
        'border shadow-sm',
        'transition-all duration-150',
        isDark ? config.darkColor : config.lightColor,
        isDragging && 'opacity-50 scale-95',
        disabled && 'opacity-50 cursor-not-allowed'
      )}
    >
      <Icon className="w-4 h-4" />
      <span className="text-[7px] font-medium leading-none mt-0.5">{config.label}</span>
    </div>
  );
}

export function NodePalette({
  onDragStart,
  onDragEnd,
  disabled = false,
  className,
}: NodePaletteProps) {
  const themeClasses = useThemeClasses();
  const paletteRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState(DEFAULT_POSITION);
  const [isDraggingPalette, setIsDraggingPalette] = useState(false);
  const dragStartPos = useRef({ x: 0, y: 0, paletteX: 0, paletteY: 0 });

  // Handle grip mouse down to start dragging palette
  const handleGripMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDraggingPalette(true);

    // Get current position
    const rect = paletteRef.current?.getBoundingClientRect();
    const parentRect = paletteRef.current?.parentElement?.getBoundingClientRect();
    if (rect && parentRect) {
      dragStartPos.current = {
        x: e.clientX,
        y: e.clientY,
        paletteX: rect.left - parentRect.left,
        paletteY: rect.top - parentRect.top,
      };
    }
  }, []);

  // Handle double-click to reset position
  const handleGripDoubleClick = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setPosition(DEFAULT_POSITION);
  }, []);

  // Handle mouse move for dragging
  useEffect(() => {
    if (!isDraggingPalette) return;

    const handleMouseMove = (e: MouseEvent) => {
      const deltaX = e.clientX - dragStartPos.current.x;
      const deltaY = e.clientY - dragStartPos.current.y;
      setPosition({
        x: dragStartPos.current.paletteX + deltaX,
        y: dragStartPos.current.paletteY + deltaY,
      });
    };

    const handleMouseUp = () => {
      setIsDraggingPalette(false);
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isDraggingPalette]);

  // Calculate style based on position
  const positionStyle: React.CSSProperties = position.y === -1
    ? { left: `${position.x}px`, top: '50%', transform: 'translateY(-50%)' }
    : { left: `${position.x}px`, top: `${position.y}px` };

  return (
    <div
      ref={paletteRef}
      className={clsx(
        'absolute z-40',
        'w-12 p-1.5 rounded-lg',
        themeClasses.isDark
          ? 'bg-black/50 backdrop-blur-md border border-white/10'
          : 'bg-white/90 backdrop-blur-md border border-gray-200',
        'shadow-lg',
        'flex flex-col items-center gap-1',
        isDraggingPalette && 'cursor-grabbing',
        disabled && 'opacity-50 pointer-events-none',
        className
      )}
      style={positionStyle}
    >
      {/* Drag handle - horizontal grip */}
      <div
        className={clsx(
          'w-full flex items-center justify-center cursor-grab active:cursor-grabbing py-0.5',
          themeClasses.isDark ? 'text-white/40 hover:text-white/60' : 'text-gray-400 hover:text-gray-600'
        )}
        onMouseDown={handleGripMouseDown}
        onDoubleClick={handleGripDoubleClick}
        title="Drag to reposition. Double-click to reset."
      >
        <GripHorizontal className="w-4 h-4" />
      </div>

      {/* Step item */}
      <PaletteItem
        config={PALETTE_ITEMS[0]}
        isDark={themeClasses.isDark}
        onDragStart={onDragStart}
        onDragEnd={onDragEnd}
        disabled={disabled}
      />

      {/* AI Agent item */}
      <PaletteItem
        config={PALETTE_ITEMS[1]}
        isDark={themeClasses.isDark}
        onDragStart={onDragStart}
        onDragEnd={onDragEnd}
        disabled={disabled}
      />

      {/* Human Task item */}
      <PaletteItem
        config={PALETTE_ITEMS[2]}
        isDark={themeClasses.isDark}
        onDragStart={onDragStart}
        onDragEnd={onDragEnd}
        disabled={disabled}
      />

      {/* Chat item */}
      <PaletteItem
        config={PALETTE_ITEMS[3]}
        isDark={themeClasses.isDark}
        onDragStart={onDragStart}
        onDragEnd={onDragEnd}
        disabled={disabled}
      />

      {/* Divider */}
      <div className={clsx(
        'w-6 h-px my-0.5',
        themeClasses.isDark ? 'bg-white/20' : 'bg-gray-300'
      )} />

      {/* Container items */}
      {PALETTE_ITEMS.slice(4).map((item) => (
        <PaletteItem
          key={item.type}
          config={item}
          isDark={themeClasses.isDark}
          onDragStart={onDragStart}
          onDragEnd={onDragEnd}
          disabled={disabled}
        />
      ))}
    </div>
  );
}
