import React, { useEffect, useState } from 'react';
import { Box, Text } from 'ink';
import Spinner from 'ink-spinner';
import type { RunEvent } from '../wasm-fn/run.js';
import type { RunOutcome } from '../wasm-fn/run-client.js';
import type { RunPlan } from '../wasm-fn/run-target.js';

/**
 * Live view of one `raisindb function run`.
 *
 * The component owns no logic beyond rendering: it is handed an `execute`
 * function, feeds it an emitter, and draws what comes back. Log lines arrive
 * from the server's SSE stream as the guest writes them, which is the whole
 * point of the run-file path — a `console.log` in a wasm handler shows up here
 * before the result does.
 */
export interface FunctionRunProps {
  /** One-line header: what is being run, and where. */
  title: string;
  /** Performs the run, emitting progress as it goes. */
  execute: (
    emit: (event: RunEvent) => void
  ) => Promise<{ outcome: RunOutcome; plan: RunPlan }>;
  /** Called once with the process exit code when the run settles. */
  onDone: (exitCode: number) => void;
}

/** Colour per log level, matching the server's `LogEntry` levels. */
const LEVEL_COLOR: Record<string, string> = {
  debug: 'gray',
  info: 'cyan',
  warn: 'yellow',
  error: 'red',
};

export function FunctionRun({ title, execute, onDone }: FunctionRunProps) {
  const [phases, setPhases] = useState<string[]>([]);
  const [logs, setLogs] = useState<{ level: string; message: string }[]>([]);
  const [outcome, setOutcome] = useState<RunOutcome | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [running, setRunning] = useState(true);

  useEffect(() => {
    let cancelled = false;
    const emit = (event: RunEvent) => {
      if (cancelled) return;
      if (event.kind === 'phase') setPhases((prev) => [...prev, event.message]);
      else if (event.kind === 'log') {
        setLogs((prev) => [...prev, { level: event.level, message: event.message }]);
      }
    };
    execute(emit)
      .then(({ outcome: result }) => {
        if (cancelled) return;
        setOutcome(result);
        setRunning(false);
        onDone(result.success ? 0 : 1);
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        setFailure(error instanceof Error ? error.message : String(error));
        setRunning(false);
        onDone(2);
      });
    return () => {
      cancelled = true;
    };
    // Run exactly once: the props are the whole invocation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <Box flexDirection="column">
      <Text bold>{title}</Text>
      {phases.map((phase, index) => (
        <Text key={`phase-${index}`} color="gray">
          {'  '}
          {phase}
        </Text>
      ))}
      {logs.length > 0 && <Text>{''}</Text>}
      {logs.map((log, index) => (
        <Text key={`log-${index}`} color={LEVEL_COLOR[log.level] || 'white'}>
          {'  '}
          {log.level.padEnd(5)} {log.message}
        </Text>
      ))}
      {running && (
        <Text color="cyan">
          <Spinner type="dots" /> running
        </Text>
      )}
      {failure && <Text color="red">{`\nx ${failure}`}</Text>}
      {outcome && !outcome.success && (
        <Text color="red">{`\nx ${outcome.error || 'the function failed'}`}</Text>
      )}
      {outcome && outcome.success && (
        <Box flexDirection="column">
          <Text color="green">
            {`\n+ ok${outcome.durationMs !== undefined ? ` in ${outcome.durationMs} ms` : ''}`}
          </Text>
          <Text>{JSON.stringify(outcome.result, null, 2)}</Text>
        </Box>
      )}
    </Box>
  );
}
