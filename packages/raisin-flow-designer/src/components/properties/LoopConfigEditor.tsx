/**
 * Loop Configuration Editor
 *
 * A loop repeats its children in one of three ways, and EXACTLY ONE of them
 * decides its shape: `over` a collection, `while` a condition holds, or a fixed
 * number of `times`. The engine refuses a config that names none or several
 * rather than picking by precedence, because guessing which an author meant is
 * how a loop silently runs the wrong number of times.
 *
 * So the shape is a SEGMENTED CHOICE, not three independent fields: switching
 * clears the other two. The invalid state simply cannot be authored here, which
 * is better than authoring it and being told off by the validator afterwards.
 *
 * This replaces a read-only `JSON.stringify` dump — the loop was the one
 * container whose config the panel showed but would not let you touch, so
 * `while`, `times` and `unbounded` were unreachable from the UI entirely.
 */

import { useCallback } from 'react';
import { clsx } from 'clsx';
import { Repeat, Infinity as InfinityIcon } from 'lucide-react';
import { useFlowDesignerContext } from '../../context/FlowDesignerContext';
import { useThemeClasses } from '../../context';
import { UpdateRulesCommand } from '../../commands';
import type { FlowContainer, LoopConfig } from '../../types';

export interface LoopConfigEditorProps {
  /** The loop container to edit */
  container: FlowContainer;
  /** Custom class name */
  className?: string;
}

type LoopShape = 'over' | 'while' | 'times';

const SHAPES: { value: LoopShape; label: string; hint: string }[] = [
  { value: 'over', label: 'For each', hint: 'Once per item of a collection' },
  { value: 'while', label: 'While', hint: 'Re-tested before each iteration' },
  { value: 'times', label: 'Times', hint: 'A fixed number of repeats' },
];

/**
 * Which shape a config currently names, or `null` when it names none.
 *
 * The null case is real and must not be papered over by defaulting to `over`:
 * a freshly-dropped loop has not chosen yet, and showing "For each" as selected
 * while `over` is unset would claim a decision the author has not made — then
 * report LOOP_MISSING_SHAPE against a form that looks complete.
 */
function shapeOf(loop: LoopConfig | undefined): LoopShape | null {
  if (loop?.while != null) return 'while';
  if (loop?.times != null) return 'times';
  if (loop?.over != null) return 'over';
  return null;
}

export function LoopConfigEditor({ container, className }: LoopConfigEditorProps) {
  const { commandContext, executeCommand } = useFlowDesignerContext();
  const themeClasses = useThemeClasses();

  const loop = container.loop ?? {};
  const shape = shapeOf(container.loop);

  const commit = useCallback(
    (next: LoopConfig) => {
      executeCommand(
        new UpdateRulesCommand(commandContext, { containerId: container.id, loop: next })
      );
    },
    [commandContext, executeCommand, container.id]
  );

  const setField = useCallback(
    <K extends keyof LoopConfig>(key: K, value: LoopConfig[K]) => {
      const next = { ...loop };
      // An empty box means "not set", not "set to empty" — leaving `over: ''`
      // behind is what made a half-edited loop look like a for_each missing its
      // collection.
      if (value === '' || value == null || (typeof value === 'number' && Number.isNaN(value))) {
        delete next[key];
      } else {
        next[key] = value;
      }
      commit(next);
    },
    [loop, commit]
  );

  /** Switching shape CLEARS the other two, so the exactly-one rule holds. */
  const setShape = useCallback(
    (nextShape: LoopShape) => {
      const { over: _o, while: _w, times: _t, unbounded: _u, ...rest } = loop;
      const next: LoopConfig = { ...rest };
      if (nextShape === 'over') next.over = loop.over ?? '';
      if (nextShape === 'while') next.while = loop.while ?? '';
      if (nextShape === 'times') next.times = loop.times ?? 1;
      // `unbounded` is meaningless off a `while`, so it does not survive the move.
      if (nextShape === 'while' && loop.unbounded) next.unbounded = true;
      commit(next);
    },
    [loop, commit]
  );

  const inputClass = clsx(
    'w-full px-3 py-2 rounded-md border text-sm',
    themeClasses.stepBg,
    themeClasses.stepBorder,
    themeClasses.stepText,
    'focus:outline-none focus:ring-2 focus:ring-blue-500'
  );

  return (
    <div className={clsx('space-y-3', className)}>
      <label
        className={clsx(
          'text-xs font-medium mb-1.5 flex items-center gap-1.5',
          themeClasses.stepTextMuted
        )}
      >
        <Repeat className="w-4 h-4" />
        Repeat
      </label>

      {/* Shape — segmented, because exactly one may be set */}
      <div role="radiogroup" aria-label="Loop shape" className="flex gap-1">
        {SHAPES.map((s) => (
          <button
            key={s.value}
            type="button"
            role="radio"
            aria-checked={shape === s.value}
            title={s.hint}
            onClick={() => setShape(s.value)}
            className={clsx(
              'flex-1 px-2 py-1.5 rounded-md border text-xs font-medium transition',
              shape === s.value
                ? 'border-blue-500 bg-blue-500/10 text-blue-600 dark:text-blue-400'
                : clsx(themeClasses.stepBorder, themeClasses.stepTextMuted, 'hover:border-blue-400')
            )}
          >
            {s.label}
          </button>
        ))}
      </div>
      <p className={clsx('text-xs -mt-1', themeClasses.stepTextMuted)}>
        {shape
          ? SHAPES.find((s) => s.value === shape)?.hint
          : 'Pick how this loop repeats — exactly one of the three.'}
      </p>

      {shape === 'over' && (
        <input
          type="text"
          value={loop.over ?? ''}
          onChange={(e) => setField('over', e.target.value)}
          placeholder="${steps.pick.items}"
          aria-label="Collection expression"
          className={clsx(inputClass, 'font-mono')}
        />
      )}

      {shape === 'while' && (
        <>
          <input
            type="text"
            value={loop.while ?? ''}
            onChange={(e) => setField('while', e.target.value)}
            placeholder="steps.critic.passed == false"
            aria-label="While condition"
            className={clsx(inputClass, 'font-mono')}
          />
          <label className={clsx('flex items-start gap-2 text-xs', themeClasses.stepText)}>
            <input
              type="checkbox"
              checked={loop.unbounded === true}
              onChange={(e) => {
                const next = { ...loop };
                if (e.target.checked) {
                  next.unbounded = true;
                  // A ceiling and no ceiling is a contradiction the engine
                  // refuses, so ticking this takes the cap with it.
                  delete next.max_iterations;
                } else {
                  delete next.unbounded;
                }
                commit(next);
              }}
              className="mt-0.5"
            />
            <span>
              <span className="inline-flex items-center gap-1 font-medium">
                <InfinityIcon className="w-3 h-3" />
                No iteration limit
              </span>
              <span className={clsx('block', themeClasses.stepTextMuted)}>
                Runs purely on the condition. Safe because the engine stops a runaway
                itself — but nothing else will end this loop.
              </span>
            </span>
          </label>
        </>
      )}

      {shape === 'times' && (
        <input
          type="number"
          min={1}
          value={loop.times ?? 1}
          onChange={(e) => setField('times', e.target.valueAsNumber)}
          aria-label="Repeat count"
          className={inputClass}
        />
      )}

      {shape && (
      <div className="grid grid-cols-2 gap-2">
        <div>
          <label className={clsx('block text-xs mb-1', themeClasses.stepTextMuted)}>
            Item variable
          </label>
          <input
            type="text"
            value={loop.item ?? ''}
            onChange={(e) => setField('item', e.target.value)}
            placeholder="item"
            className={clsx(inputClass, 'font-mono')}
          />
        </div>
        <div>
          <label className={clsx('block text-xs mb-1', themeClasses.stepTextMuted)}>
            Index variable
          </label>
          <input
            type="text"
            value={loop.index ?? ''}
            onChange={(e) => setField('index', e.target.value)}
            placeholder="(none)"
            className={clsx(inputClass, 'font-mono')}
          />
        </div>
      </div>

      )}

      {/* A `times` loop's count IS its ceiling, and an unbounded one has none. */}
      {shape != null && !(shape === 'times' || loop.unbounded === true) && (
        <div>
          <label className={clsx('block text-xs mb-1', themeClasses.stepTextMuted)}>
            Max iterations
          </label>
          <input
            type="number"
            min={1}
            value={loop.max_iterations ?? ''}
            onChange={(e) => setField('max_iterations', e.target.valueAsNumber)}
            placeholder={shape === 'while' ? '1000' : '(no cap)'}
            className={inputClass}
          />
        </div>
      )}

      {shape && (
      <div>
        <label className={clsx('block text-xs mb-1', themeClasses.stepTextMuted)}>
          Stop early when
        </label>
        <input
          type="text"
          value={loop.until ?? ''}
          onChange={(e) => setField('until', e.target.value)}
          placeholder="steps.ask.response == 'accept'"
          className={clsx(inputClass, 'font-mono')}
        />
        <p className={clsx('text-xs mt-1', themeClasses.stepTextMuted)}>
          Checked after each completed iteration.
        </p>
      </div>
      )}
    </div>
  );
}
