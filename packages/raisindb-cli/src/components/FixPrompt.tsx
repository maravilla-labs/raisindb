/**
 * Interactive component for fixing validation errors
 */

import React, { useState } from 'react';
import { Box, Text, useInput } from 'ink';
import SelectInput from 'ink-select-input';
import TextInput from 'ink-text-input';
import type { ValidationError } from '../wasm/types.js';

interface FixPromptProps {
  error: ValidationError;
  currentIndex: number;
  totalCount: number;
  onAccept: (newValue?: string) => void;
  onSkip: () => void;
  onSkipAll: () => void;
}

type Mode = 'select' | 'input';

/**
 * Interactive prompt for fixing a validation error
 */
export const FixPrompt: React.FC<FixPromptProps> = ({
  error,
  currentIndex,
  totalCount,
  onAccept,
  onSkip,
  onSkipAll,
}) => {
  const [mode, setMode] = useState<Mode>('select');
  const [inputValue, setInputValue] = useState('');

  const fix = error.suggested_fix;

  // Build selection items based on fix type
  const buildItems = () => {
    const items: Array<{ label: string; value: string }> = [];

    if (error.fix_type === 'auto_fixable' && fix?.new_value) {
      items.push({
        label: `Apply fix: ${fix.new_value}`,
        value: 'accept',
      });
    }

    if (error.fix_type === 'needs_input' && fix?.options) {
      for (const option of fix.options) {
        items.push({
          label: option,
          value: `option:${option}`,
        });
      }
      items.push({
        label: 'Enter custom value...',
        value: 'custom',
      });
    }

    items.push({ label: 'Skip this error', value: 'skip' });
    items.push({ label: 'Skip all remaining errors', value: 'skip_all' });

    return items;
  };

  const handleSelect = (item: { label: string; value: string }) => {
    if (item.value === 'accept') {
      onAccept(fix?.new_value);
    } else if (item.value === 'skip') {
      onSkip();
    } else if (item.value === 'skip_all') {
      onSkipAll();
    } else if (item.value === 'custom') {
      setMode('input');
    } else if (item.value.startsWith('option:')) {
      const value = item.value.substring(7);
      onAccept(value);
    }
  };

  const handleInputSubmit = () => {
    if (inputValue.trim()) {
      onAccept(inputValue.trim());
    }
  };

  // Handle escape to go back to select mode
  useInput((input, key) => {
    if (key.escape && mode === 'input') {
      setMode('select');
      setInputValue('');
    }
  });

  return (
    <Box flexDirection="column" borderStyle="round" borderColor="yellow" padding={1}>
      {/* Header */}
      <Box marginBottom={1}>
        <Text bold color="yellow">
          Fix {currentIndex + 1} of {totalCount}
        </Text>
      </Box>

      {/* Error info */}
      <Box flexDirection="column" marginBottom={1}>
        <Box>
          <Text color="red">Error: </Text>
          <Text>{error.message}</Text>
        </Box>
        <Box>
          <Text dimColor>File: </Text>
          <Text>{error.file_path}</Text>
        </Box>
        {error.field_path && (
          <Box>
            <Text dimColor>Field: </Text>
            <Text>{error.field_path}</Text>
          </Box>
        )}
      </Box>

      {/* Fix suggestion */}
      {fix && (
        <Box flexDirection="column" marginBottom={1}>
          <Text color="cyan">{fix.description}</Text>
          {fix.old_value && fix.new_value && (
            <Box>
              <Text dimColor>Change: </Text>
              <Text color="red">{fix.old_value}</Text>
              <Text dimColor> -&gt; </Text>
              <Text color="green">{fix.new_value}</Text>
            </Box>
          )}
        </Box>
      )}

      {/* Selection or input mode */}
      {mode === 'select' ? (
        <SelectInput items={buildItems()} onSelect={handleSelect} />
      ) : (
        <Box flexDirection="column">
          <Text>Enter value (Esc to cancel):</Text>
          <Box>
            <Text color="green">&gt; </Text>
            <TextInput
              value={inputValue}
              onChange={setInputValue}
              onSubmit={handleInputSubmit}
            />
          </Box>
        </Box>
      )}
    </Box>
  );
};

/**
 * Component to handle fixing multiple errors interactively
 */
interface FixFlowProps {
  errors: ValidationError[];
  onFix: (error: ValidationError, newValue?: string) => Promise<void>;
  onComplete: () => void;
}

export const FixFlow: React.FC<FixFlowProps> = ({ errors, onFix, onComplete }) => {
  const [currentIndex, setCurrentIndex] = useState(0);
  const [isProcessing, setIsProcessing] = useState(false);

  const fixableErrors = errors.filter(
    (e) => e.fix_type === 'auto_fixable' || e.fix_type === 'needs_input'
  );

  if (fixableErrors.length === 0 || currentIndex >= fixableErrors.length) {
    // No more errors to fix
    React.useEffect(() => {
      onComplete();
    }, [onComplete]);
    return null;
  }

  const currentError = fixableErrors[currentIndex];

  const handleAccept = async (newValue?: string) => {
    setIsProcessing(true);
    try {
      await onFix(currentError, newValue);
    } catch (error) {
      // Continue even if fix fails
    }
    setIsProcessing(false);
    setCurrentIndex((i) => i + 1);
  };

  const handleSkip = () => {
    setCurrentIndex((i) => i + 1);
  };

  const handleSkipAll = () => {
    setCurrentIndex(fixableErrors.length);
  };

  if (isProcessing) {
    return (
      <Box>
        <Text color="cyan">Applying fix...</Text>
      </Box>
    );
  }

  return (
    <FixPrompt
      error={currentError}
      currentIndex={currentIndex}
      totalCount={fixableErrors.length}
      onAccept={handleAccept}
      onSkip={handleSkip}
      onSkipAll={handleSkipAll}
    />
  );
};

export default FixPrompt;
