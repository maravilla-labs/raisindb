import React, { useState, useEffect } from 'react';
import { Box, Text } from 'ink';
import Gradient from 'ink-gradient';
import InkSpinner from 'ink-spinner';

const BRAND_COLORS = ['#B8754E', '#D97706', '#EA580C', '#f97316'];

export type InstallPhase =
  | 'resolving'
  | 'downloading'
  | 'verifying'
  | 'extracting'
  | 'complete'
  | 'already-installed'
  | 'error';

export interface InstallState {
  phase: InstallPhase;
  version?: string;
  target?: string;
  downloadedBytes?: number;
  totalBytes?: number;
  installPath?: string;
  error?: string;
}

function ProgressBar({ current, total, width = 30 }: { current: number; total: number; width?: number }) {
  const pct = total > 0 ? Math.min(current / total, 1) : 0;
  const filled = Math.round(pct * width);
  const bar = '█'.repeat(filled) + '░'.repeat(width - filled);
  const mb = (current / 1024 / 1024).toFixed(1);
  const totalMb = (total / 1024 / 1024).toFixed(1);
  return (
    <Box>
      <Text color="cyan">{bar}</Text>
      <Text> {mb}MB / {totalMb}MB</Text>
      <Text dimColor> ({Math.round(pct * 100)}%)</Text>
    </Box>
  );
}

export function ServerInstallUI({ state }: { state: InstallState }) {
  const { phase, version, target, downloadedBytes = 0, totalBytes = 0, installPath, error } = state;

  return (
    <Box flexDirection="column" paddingX={1} paddingY={1}>
      {/* Header */}
      <Box marginBottom={1}>
        <Gradient colors={BRAND_COLORS}>
          <Text bold> RaisinDB Server </Text>
        </Gradient>
      </Box>

      {/* Already installed */}
      {phase === 'already-installed' && (
        <Box flexDirection="column">
          <Box>
            <Text color="green">✓</Text>
            <Text> Already installed: </Text>
            <Text bold color="white">v{version}</Text>
          </Box>
          {installPath && (
            <Box marginTop={0}>
              <Text dimColor>  {installPath}</Text>
            </Box>
          )}
        </Box>
      )}

      {/* Resolving */}
      {phase === 'resolving' && (
        <Box>
          <Text color="cyan"><InkSpinner type="dots" /></Text>
          <Text> Resolving latest release...</Text>
        </Box>
      )}

      {/* Downloading */}
      {phase === 'downloading' && (
        <Box flexDirection="column">
          <Box marginBottom={1}>
            <Text color="cyan"><InkSpinner type="dots" /></Text>
            <Text> Downloading </Text>
            <Text bold color="white">{version}</Text>
            <Text dimColor> for {target}</Text>
          </Box>
          {totalBytes > 0 ? (
            <Box paddingLeft={2}>
              <ProgressBar current={downloadedBytes} total={totalBytes} />
            </Box>
          ) : (
            <Box paddingLeft={2}>
              <Text dimColor>{(downloadedBytes / 1024 / 1024).toFixed(1)}MB downloaded...</Text>
            </Box>
          )}
        </Box>
      )}

      {/* Verifying */}
      {phase === 'verifying' && (
        <Box>
          <Text color="cyan"><InkSpinner type="dots" /></Text>
          <Text> Verifying checksum...</Text>
        </Box>
      )}

      {/* Extracting */}
      {phase === 'extracting' && (
        <Box>
          <Text color="cyan"><InkSpinner type="dots" /></Text>
          <Text> Extracting binary...</Text>
        </Box>
      )}

      {/* Complete */}
      {phase === 'complete' && (
        <Box flexDirection="column">
          <Box>
            <Text color="green">✓</Text>
            <Text> Installed </Text>
            <Text bold color="white">raisindb {version}</Text>
          </Box>
          {installPath && (
            <Box marginTop={0}>
              <Text dimColor>  {installPath}</Text>
            </Box>
          )}
        </Box>
      )}

      {/* Error */}
      {phase === 'error' && (
        <Box flexDirection="column">
          <Box>
            <Text color="red">✗</Text>
            <Text color="red"> Installation failed</Text>
          </Box>
          {error && (
            <Box marginTop={0} paddingLeft={2}>
              <Text color="red">{error}</Text>
            </Box>
          )}
        </Box>
      )}
    </Box>
  );
}

export function ServerStartUI({ state, starting }: { state: InstallState; starting: boolean }) {
  return (
    <Box flexDirection="column">
      {/* Show install progress if needed */}
      {state.phase !== 'complete' && state.phase !== 'already-installed' && (
        <ServerInstallUI state={state} />
      )}

      {/* Show install complete briefly, then starting */}
      {(state.phase === 'complete' || state.phase === 'already-installed') && !starting && (
        <ServerInstallUI state={state} />
      )}

      {/* Starting */}
      {starting && (
        <Box flexDirection="column" paddingX={1} paddingY={1}>
          <Box marginBottom={1}>
            <Gradient colors={BRAND_COLORS}>
              <Text bold> RaisinDB Server </Text>
            </Gradient>
            {state.version && <Text dimColor> v{state.version}</Text>}
          </Box>
          <Box>
            <Text color="green">▸</Text>
            <Text> Server starting...</Text>
          </Box>
        </Box>
      )}
    </Box>
  );
}
