/**
 * Sync progress display component
 */

import React from 'react';
import { Box, Text } from 'ink';
import Spinner from 'ink-spinner';

export type SyncPhase =
  | 'comparing'
  | 'uploading'
  | 'downloading'
  | 'resolving'
  | 'complete'
  | 'error';

export interface SyncProgressProps {
  phase: SyncPhase;
  current: number;
  total: number;
  currentFile?: string;
  conflicts?: number;
  uploaded?: number;
  downloaded?: number;
  message?: string;
  error?: string;
}

export function SyncProgress({
  phase,
  current,
  total,
  currentFile,
  conflicts = 0,
  uploaded = 0,
  downloaded = 0,
  message,
  error,
}: SyncProgressProps): React.ReactElement {
  const progress = total > 0 ? Math.round((current / total) * 100) : 0;
  const progressBarWidth = 40;
  const filledWidth = Math.round((progress / 100) * progressBarWidth);
  const progressBar = '█'.repeat(filledWidth) + '░'.repeat(progressBarWidth - filledWidth);

  const getPhaseText = (): string => {
    switch (phase) {
      case 'comparing':
        return 'Comparing files...';
      case 'uploading':
        return 'Uploading changes...';
      case 'downloading':
        return 'Downloading changes...';
      case 'resolving':
        return 'Resolving conflicts...';
      case 'complete':
        return 'Sync complete!';
      case 'error':
        return 'Sync failed';
    }
  };

  const getPhaseColor = (): string => {
    switch (phase) {
      case 'comparing':
        return 'cyan';
      case 'uploading':
        return 'yellow';
      case 'downloading':
        return 'blue';
      case 'resolving':
        return 'magenta';
      case 'complete':
        return 'green';
      case 'error':
        return 'red';
    }
  };

  return (
    <Box flexDirection="column" padding={1}>
      <Box marginBottom={1}>
        {phase !== 'complete' && phase !== 'error' && (
          <Text color="cyan">
            <Spinner type="dots" />
          </Text>
        )}
        <Text bold color={getPhaseColor()}>
          {' '}
          Syncing Package
        </Text>
      </Box>

      <Box marginBottom={1}>
        <Text>Phase: </Text>
        <Text color={getPhaseColor()}>{getPhaseText()}</Text>
      </Box>

      {phase !== 'complete' && phase !== 'error' && (
        <>
          <Box marginBottom={1}>
            <Text color="gray">[</Text>
            <Text color="cyan">{progressBar}</Text>
            <Text color="gray">]</Text>
            <Text> {progress}%</Text>
          </Box>

          {currentFile && (
            <Box marginBottom={1}>
              <Text>Current: </Text>
              <Text color="white">{currentFile}</Text>
            </Box>
          )}
        </>
      )}

      <Box marginTop={1}>
        <Text color="green">↑ {uploaded} uploaded</Text>
        <Text>  </Text>
        <Text color="blue">↓ {downloaded} downloaded</Text>
        {conflicts > 0 && (
          <>
            <Text>  </Text>
            <Text color="yellow">⚠ {conflicts} conflict{conflicts !== 1 ? 's' : ''}</Text>
          </>
        )}
      </Box>

      {message && (
        <Box marginTop={1}>
          <Text color="gray">{message}</Text>
        </Box>
      )}

      {error && (
        <Box marginTop={1}>
          <Text color="red">Error: {error}</Text>
        </Box>
      )}
    </Box>
  );
}

/**
 * Conflict resolver component props
 */
export interface ConflictResolverProps {
  filePath: string;
  localMtime?: Date;
  serverMtime?: Date;
  onResolve: (resolution: 'local' | 'server' | 'skip') => void;
}

/**
 * Simple conflict display (full interactive version would use SelectInput)
 */
export function ConflictDisplay({
  filePath,
  localMtime,
  serverMtime,
}: Omit<ConflictResolverProps, 'onResolve'>): React.ReactElement {
  return (
    <Box flexDirection="column" borderStyle="single" borderColor="yellow" padding={1}>
      <Text bold color="yellow">
        ⚠ Conflict: {filePath}
      </Text>
      <Box marginTop={1}>
        <Text>Local: Modified {localMtime?.toLocaleString() || 'unknown'}</Text>
      </Box>
      <Box>
        <Text>Server: Modified {serverMtime?.toLocaleString() || 'unknown'}</Text>
      </Box>
    </Box>
  );
}
