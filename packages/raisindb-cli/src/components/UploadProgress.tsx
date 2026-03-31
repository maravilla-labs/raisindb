import React, { useState, useEffect } from 'react';
import { Box, Text } from 'ink';
import Spinner from 'ink-spinner';
import Gradient from 'ink-gradient';

interface UploadProgressProps {
  /** Current phase: 'uploading' | 'processing' | 'complete' | 'error' */
  phase: 'uploading' | 'processing' | 'complete' | 'error';
  /** Progress percentage (0-100) */
  progress: number;
  /** File name being uploaded */
  fileName: string;
  /** Status message to display */
  message?: string;
  /** Error message if phase is 'error' */
  error?: string;
}

// Gradient colors for the animated text
const GRADIENT_COLORS = ['#FF6B6B', '#4ECDC4', '#45B7D1', '#96E6A1', '#DDA0DD', '#FF6B6B'];

// Progress bar characters
const FILLED_CHAR = '\u2588'; // Full block
const EMPTY_CHAR = '\u2591';  // Light shade

export function UploadProgress({
  phase,
  progress,
  fileName,
  message,
  error,
}: UploadProgressProps) {
  // Animated gradient offset for the text
  const [gradientOffset, setGradientOffset] = useState(0);

  useEffect(() => {
    if (phase === 'uploading' || phase === 'processing') {
      const interval = setInterval(() => {
        setGradientOffset((prev) => (prev + 1) % GRADIENT_COLORS.length);
      }, 200);
      return () => clearInterval(interval);
    }
  }, [phase]);

  // Shift gradient colors for animation effect
  const shiftedColors = [
    ...GRADIENT_COLORS.slice(gradientOffset),
    ...GRADIENT_COLORS.slice(0, gradientOffset),
  ];

  // Build progress bar (40 characters wide)
  const barWidth = 40;
  const filledWidth = Math.round((progress / 100) * barWidth);
  const emptyWidth = barWidth - filledWidth;
  const progressBar = FILLED_CHAR.repeat(filledWidth) + EMPTY_CHAR.repeat(emptyWidth);

  // Phase-specific status text
  const getPhaseText = () => {
    switch (phase) {
      case 'uploading':
        return 'Uploading Package';
      case 'processing':
        return 'Processing Package';
      case 'complete':
        return 'Upload Complete';
      case 'error':
        return 'Upload Failed';
    }
  };

  // Phase-specific colors
  const getStatusColor = () => {
    switch (phase) {
      case 'uploading':
      case 'processing':
        return 'cyan';
      case 'complete':
        return 'green';
      case 'error':
        return 'red';
    }
  };

  return (
    <Box flexDirection="column" marginY={1}>
      {/* Animated gradient title for active phases */}
      <Box marginBottom={1}>
        {(phase === 'uploading' || phase === 'processing') ? (
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
            {'\u2714'} {getPhaseText()}
          </Text>
        ) : (
          <Text color="red" bold>
            {'\u2718'} {getPhaseText()}
          </Text>
        )}
      </Box>

      {/* File name */}
      <Box>
        <Text dimColor>File: </Text>
        <Text color="white">{fileName}</Text>
      </Box>

      {/* Progress bar for active phases */}
      {(phase === 'uploading' || phase === 'processing') && (
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

      {/* Success message */}
      {phase === 'complete' && message && (
        <Box marginTop={1}>
          <Text color="green">{message}</Text>
        </Box>
      )}

      {/* Error message */}
      {phase === 'error' && error && (
        <Box marginTop={1}>
          <Text color="red">{error}</Text>
        </Box>
      )}
    </Box>
  );
}

export default UploadProgress;
