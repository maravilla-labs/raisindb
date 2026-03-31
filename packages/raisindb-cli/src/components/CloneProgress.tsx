/**
 * Clone progress display component
 */

import React, { useState, useEffect } from 'react';
import { Box, Text } from 'ink';
import Spinner from 'ink-spinner';
import Gradient from 'ink-gradient';

export type ClonePhase =
  | 'preparing'
  | 'exporting'
  | 'downloading'
  | 'extracting'
  | 'complete'
  | 'error';

export interface CloneProgressProps {
  phase: ClonePhase;
  packageName: string;
  progress?: number;
  message?: string;
  error?: string;
  outputDir?: string;
}

const GRADIENT_COLORS = ['#4ECDC4', '#45B7D1', '#96E6A1', '#DDA0DD', '#4ECDC4'];
const FILLED_CHAR = '\u2588';
const EMPTY_CHAR = '\u2591';

export function CloneProgress({
  phase,
  packageName,
  progress = 0,
  message,
  error,
  outputDir,
}: CloneProgressProps) {
  const [gradientOffset, setGradientOffset] = useState(0);

  useEffect(() => {
    if (phase !== 'complete' && phase !== 'error') {
      const interval = setInterval(() => {
        setGradientOffset((prev) => (prev + 1) % GRADIENT_COLORS.length);
      }, 200);
      return () => clearInterval(interval);
    }
  }, [phase]);

  const shiftedColors = [
    ...GRADIENT_COLORS.slice(gradientOffset),
    ...GRADIENT_COLORS.slice(0, gradientOffset),
  ];

  const barWidth = 40;
  const filledWidth = Math.round((progress / 100) * barWidth);
  const progressBar = FILLED_CHAR.repeat(filledWidth) + EMPTY_CHAR.repeat(barWidth - filledWidth);

  const getPhaseText = (): string => {
    switch (phase) {
      case 'preparing':
        return 'Preparing Export';
      case 'exporting':
        return 'Exporting Package';
      case 'downloading':
        return 'Downloading Package';
      case 'extracting':
        return 'Extracting Files';
      case 'complete':
        return 'Clone Complete';
      case 'error':
        return 'Clone Failed';
    }
  };

  const getPhaseIcon = (): string => {
    switch (phase) {
      case 'complete':
        return '\u2714'; // Check mark
      case 'error':
        return '\u2718'; // X mark
      default:
        return '';
    }
  };

  const isActive = phase !== 'complete' && phase !== 'error';

  return (
    <Box flexDirection="column" marginY={1}>
      {/* Header */}
      <Box marginBottom={1}>
        {isActive ? (
          <Box>
            <Text color="cyan">
              <Spinner type="dots" />
            </Text>
            <Text> </Text>
            <Gradient colors={shiftedColors}>
              {getPhaseText()}
            </Gradient>
          </Box>
        ) : phase === 'complete' ? (
          <Text color="green" bold>
            {getPhaseIcon()} {getPhaseText()}
          </Text>
        ) : (
          <Text color="red" bold>
            {getPhaseIcon()} {getPhaseText()}
          </Text>
        )}
      </Box>

      {/* Package name */}
      <Box>
        <Text dimColor>Package: </Text>
        <Text color="white" bold>{packageName}</Text>
      </Box>

      {/* Progress bar for active phases */}
      {isActive && (
        <Box flexDirection="column" marginTop={1}>
          <Box>
            <Text color="cyan">[</Text>
            <Gradient colors={['#4ECDC4', '#45B7D1', '#96E6A1']}>
              {progressBar}
            </Gradient>
            <Text color="cyan">]</Text>
            <Text> </Text>
            <Text color="white" bold>{progress}%</Text>
          </Box>
          {message && (
            <Box marginTop={1}>
              <Text dimColor>{message}</Text>
            </Box>
          )}
        </Box>
      )}

      {/* Success output */}
      {phase === 'complete' && outputDir && (
        <Box flexDirection="column" marginTop={1}>
          <Box>
            <Text color="green">Package cloned to: </Text>
            <Text color="white" bold>{outputDir}</Text>
          </Box>
          <Box marginTop={1}>
            <Text dimColor>You can now:</Text>
          </Box>
          <Box>
            <Text dimColor>  cd {outputDir}</Text>
          </Box>
          <Box>
            <Text dimColor>  raisindb package sync --watch</Text>
          </Box>
        </Box>
      )}

      {/* Error output */}
      {phase === 'error' && error && (
        <Box marginTop={1}>
          <Text color="red">{error}</Text>
        </Box>
      )}
    </Box>
  );
}

export default CloneProgress;
