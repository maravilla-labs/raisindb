/**
 * Empty Drop Zone Component
 *
 * Placeholder shown in empty containers for dropping items.
 */

import { clsx } from 'clsx';
import { Plus } from 'lucide-react';

export interface EmptyDropZoneProps {
  /** Parent container ID */
  containerId: string;
  /** Whether the zone is highlighted for drop */
  highlighted?: boolean;
  /** Click handler to add item */
  onAdd?: () => void;
  /** Custom class name */
  className?: string;
}

export function EmptyDropZone({
  containerId,
  highlighted = false,
  onAdd,
  className,
}: EmptyDropZoneProps) {
  return (
    <div
      data-flow-drop-zone={containerId}
      className={clsx(
        'flex flex-col items-center justify-center',
        'min-h-[100px] min-w-[200px] p-4',
        'border-2 border-dashed rounded-lg',
        'transition-colors',
        highlighted
          ? 'border-sky-400 bg-sky-500/10 text-white'
          : 'border-sky-700/50 bg-white/5 hover:border-sky-400/60 text-slate-200',
        className
      )}
    >
      <div className="text-center">
        <p className="text-sm text-gray-500 mb-2">Drop items here</p>
        {onAdd && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              onAdd();
            }}
            className={clsx(
              'inline-flex items-center gap-1.5 px-3 py-1.5',
              'text-sm text-sky-300 hover:text-white',
              'bg-sky-500/10 hover:bg-sky-500/20 rounded-lg',
              'transition-colors'
            )}
          >
            <Plus className="w-4 h-4" />
            Add step
          </button>
        )}
      </div>
    </div>
  );
}
