/**
 * Enhanced package validation component with progress bar and detailed stats
 */

import React, { useState, useEffect } from 'react';
import { Box, Text } from 'ink';
import Spinner from 'ink-spinner';
import Gradient from 'ink-gradient';
import type { PackageValidationResults, ValidationError, FileType } from '../wasm/types.js';
import { extractLocales } from '../wasm/translation-validator.js';

interface PackageValidatorProps {
  /** Current phase of validation */
  phase: 'collecting' | 'validating' | 'complete' | 'error';
  /** Files collected (for progress) */
  filesCollected?: number;
  /** Current file being validated */
  currentFile?: string;
  /** Progress (0-100) */
  progress?: number;
  /** Validation results when complete */
  results?: PackageValidationResults;
  /** Original file contents for code snippets */
  fileContents?: Record<string, string>;
  /** Error message if failed */
  error?: string;
}

// Icons for different states
const ICONS = {
  success: '\u2714', // ✔
  error: '\u2718',   // ✘
  warning: '\u26A0', // ⚠
  file: '\u2022',    // •
};

// File type display names and icons
const FILE_TYPE_INFO: Record<FileType, { name: string; icon: string; color: string }> = {
  manifest: { name: 'Manifest', icon: '\u{1F4E6}', color: 'blue' },      // 📦
  nodetype: { name: 'NodeTypes', icon: '\u{1F3F7}', color: 'cyan' },    // 🏷
  workspace: { name: 'Workspaces', icon: '\u{1F4C1}', color: 'yellow' }, // 📁
  content: { name: 'Content', icon: '\u{1F4C4}', color: 'white' },      // 📄
  archetype: { name: 'Archetypes', icon: '\u{1F3A8}', color: 'magenta' }, // 🎨
  elementtype: { name: 'Elements', icon: '\u{1F9E9}', color: 'green' },  // 🧩
  translation: { name: 'Translations', icon: '\u{1F310}', color: 'cyan' }, // 🌐
};

// Progress bar characters
const FILLED_CHAR = '\u2588'; // Full block
const EMPTY_CHAR = '\u2591';  // Light shade

// YAML syntax highlighting colors
const YAML_COLORS = {
  key: '#61AFEF',      // Blue for keys
  string: '#98C379',   // Green for strings
  number: '#D19A66',   // Orange for numbers
  boolean: '#E5C07B',  // Yellow for booleans
  null: '#ABB2BF',     // Gray for null
  comment: '#5C6370',  // Dim for comments
  punctuation: '#ABB2BF', // Gray for : and -
};

interface FileTypeStats {
  count: number;
  errors: number;
  warnings: number;
}

/**
 * Parse line and column from serde_yaml error message
 */
function parseErrorLocation(message: string): { line: number; column: number } | null {
  // Match patterns like "at line 3 column 5" or "line 3 column 5"
  const match = message.match(/(?:at )?line (\d+) column (\d+)/i);
  if (match) {
    return {
      line: parseInt(match[1], 10),
      column: parseInt(match[2], 10),
    };
  }
  return null;
}

/**
 * Simple YAML syntax highlighting for a single line
 */
function highlightYamlLine(line: string): React.ReactNode[] {
  const parts: React.ReactNode[] = [];
  let remaining = line;
  let keyIndex = 0;

  // Handle comments
  if (remaining.trimStart().startsWith('#')) {
    return [<Text key="comment" color={YAML_COLORS.comment}>{line}</Text>];
  }

  // Handle list items
  const listMatch = remaining.match(/^(\s*)(- )(.*)/);
  if (listMatch) {
    parts.push(<Text key="indent">{listMatch[1]}</Text>);
    parts.push(<Text key="dash" color={YAML_COLORS.punctuation}>- </Text>);
    remaining = listMatch[3];
  }

  // Handle key: value pairs
  const keyValueMatch = remaining.match(/^(\s*)([a-zA-Z_$][a-zA-Z0-9_$]*)(:\s*)(.*)/);
  if (keyValueMatch) {
    if (!listMatch) {
      parts.push(<Text key="indent2">{keyValueMatch[1]}</Text>);
    }
    parts.push(<Text key="key" color={YAML_COLORS.key}>{keyValueMatch[2]}</Text>);
    parts.push(<Text key="colon" color={YAML_COLORS.punctuation}>{keyValueMatch[3]}</Text>);

    const value = keyValueMatch[4];
    if (value) {
      // Determine value type and color
      if (value.startsWith('"') || value.startsWith("'")) {
        parts.push(<Text key="value" color={YAML_COLORS.string}>{value}</Text>);
      } else if (/^-?\d+(\.\d+)?$/.test(value)) {
        parts.push(<Text key="value" color={YAML_COLORS.number}>{value}</Text>);
      } else if (/^(true|false)$/i.test(value)) {
        parts.push(<Text key="value" color={YAML_COLORS.boolean}>{value}</Text>);
      } else if (/^(null|~)$/i.test(value)) {
        parts.push(<Text key="value" color={YAML_COLORS.null}>{value}</Text>);
      } else {
        // Unquoted string or other value
        parts.push(<Text key="value" color={YAML_COLORS.string}>{value}</Text>);
      }
    }
    return parts;
  }

  // Just return the line as-is if no pattern matches
  return [<Text key="plain">{line}</Text>];
}

/**
 * Code snippet component with syntax highlighting
 */
function CodeSnippet({
  content,
  errorLine,
  errorColumn,
  contextLines = 2,
}: {
  content: string;
  errorLine: number;
  errorColumn: number;
  contextLines?: number;
}) {
  const lines = content.split('\n');
  const startLine = Math.max(0, errorLine - 1 - contextLines);
  const endLine = Math.min(lines.length, errorLine + contextLines);

  // Calculate the width needed for line numbers
  const lineNumWidth = String(endLine).length;

  return (
    <Box flexDirection="column" marginLeft={4} marginY={1}>
      {/* Top border */}
      <Box>
        <Text dimColor>{'─'.repeat(50)}</Text>
      </Box>

      {/* Code lines */}
      {lines.slice(startLine, endLine).map((line, idx) => {
        const actualLineNum = startLine + idx + 1;
        const isErrorLine = actualLineNum === errorLine;
        const lineNumStr = String(actualLineNum).padStart(lineNumWidth, ' ');

        return (
          <Box key={actualLineNum} flexDirection="column">
            <Box>
              {/* Line number */}
              <Text color={isErrorLine ? 'red' : 'gray'}>
                {lineNumStr}
              </Text>
              <Text color={isErrorLine ? 'red' : 'gray'}> │ </Text>

              {/* Code with syntax highlighting */}
              {isErrorLine ? (
                <Text color="white" bold>{line}</Text>
              ) : (
                highlightYamlLine(line)
              )}
            </Box>

            {/* Error pointer line */}
            {isErrorLine && (
              <Box>
                <Text color="gray">{' '.repeat(lineNumWidth)} │ </Text>
                <Text color="red">
                  {' '.repeat(Math.max(0, errorColumn - 1))}
                  {'^'.repeat(Math.min(5, line.length - errorColumn + 2 || 1))}
                </Text>
              </Box>
            )}
          </Box>
        );
      })}

      {/* Bottom border */}
      <Box>
        <Text dimColor>{'─'.repeat(50)}</Text>
      </Box>
    </Box>
  );
}

/**
 * Get stats grouped by file type
 */
function getFileTypeStats(results: PackageValidationResults): Map<FileType, FileTypeStats> {
  const stats = new Map<FileType, FileTypeStats>();

  for (const result of Object.values(results)) {
    const current = stats.get(result.file_type) || { count: 0, errors: 0, warnings: 0 };
    current.count++;
    current.errors += result.errors.length;
    current.warnings += result.warnings.length;
    stats.set(result.file_type, current);
  }

  return stats;
}

/**
 * Progress bar component
 */
function ProgressBar({ progress, width = 30 }: { progress: number; width?: number }) {
  const filledWidth = Math.round((progress / 100) * width);
  const emptyWidth = width - filledWidth;
  const progressBar = FILLED_CHAR.repeat(filledWidth) + EMPTY_CHAR.repeat(emptyWidth);

  return (
    <Box>
      <Text color="cyan">[</Text>
      <Gradient colors={['#4ECDC4', '#45B7D1', '#96E6A1']}>
        {progressBar}
      </Gradient>
      <Text color="cyan">]</Text>
      <Text> </Text>
      <Text color="white" bold>{progress}%</Text>
    </Box>
  );
}

/**
 * File type stats row
 */
function FileTypeRow({
  fileType,
  stats
}: {
  fileType: FileType;
  stats: FileTypeStats;
}) {
  const info = FILE_TYPE_INFO[fileType];
  const hasErrors = stats.errors > 0;
  const hasWarnings = stats.warnings > 0;

  return (
    <Box>
      <Box width={14}>
        <Text color={info.color as any}>{info.icon} {info.name}</Text>
      </Box>
      <Box width={8}>
        <Text dimColor>{stats.count} file{stats.count !== 1 ? 's' : ''}</Text>
      </Box>
      <Box>
        {hasErrors ? (
          <Text color="red">{stats.errors} error{stats.errors !== 1 ? 's' : ''}</Text>
        ) : hasWarnings ? (
          <Text color="yellow">{stats.warnings} warning{stats.warnings !== 1 ? 's' : ''}</Text>
        ) : (
          <Text color="green">{ICONS.success}</Text>
        )}
      </Box>
    </Box>
  );
}

/**
 * Error display with code snippet
 */
function ErrorDisplay({
  filePath,
  error,
  fileContent,
}: {
  filePath: string;
  error: ValidationError;
  fileContent?: string;
}) {
  const location = error.line && error.column
    ? { line: error.line, column: error.column }
    : parseErrorLocation(error.message);

  // Clean up message by removing the location part
  const cleanMessage = error.message
    .replace(/\s*at line \d+ column \d+\s*/gi, ' ')
    .replace(/\s*line \d+ column \d+\s*/gi, ' ')
    .trim();

  return (
    <Box flexDirection="column" marginBottom={1}>
      {/* Error header */}
      <Box>
        <Text color="red" bold>error</Text>
        <Text color="yellow">[{error.error_code}]</Text>
        <Text>: {cleanMessage}</Text>
      </Box>

      {/* File location */}
      <Box marginLeft={2}>
        <Text color="cyan">--{'>'} </Text>
        <Text color="cyan">{filePath}</Text>
        {location && (
          <Text color="cyan">:{location.line}:{location.column}</Text>
        )}
      </Box>

      {/* Code snippet */}
      {fileContent && location && (
        <CodeSnippet
          content={fileContent}
          errorLine={location.line}
          errorColumn={location.column}
        />
      )}

      {/* Help hint if available */}
      {error.suggested_fix && (
        <Box marginLeft={2} marginTop={1}>
          <Text color="green">help: </Text>
          <Text>{error.suggested_fix.description}</Text>
          {error.suggested_fix.new_value && (
            <Text color="green"> `{error.suggested_fix.new_value}`</Text>
          )}
        </Box>
      )}
    </Box>
  );
}

/**
 * Main package validator component
 */
export function PackageValidator({
  phase,
  filesCollected = 0,
  currentFile,
  progress = 0,
  results,
  fileContents,
  error,
}: PackageValidatorProps) {
  const [animFrame, setAnimFrame] = useState(0);

  // Animation for active phases
  useEffect(() => {
    if (phase === 'collecting' || phase === 'validating') {
      const interval = setInterval(() => {
        setAnimFrame((prev) => (prev + 1) % 4);
      }, 150);
      return () => clearInterval(interval);
    }
  }, [phase]);

  // Calculate stats from results
  const fileTypeStats = results ? getFileTypeStats(results) : null;
  const totalFiles = results ? Object.keys(results).length : 0;
  const totalErrors = results
    ? Object.values(results).reduce((sum, r) => sum + r.errors.length, 0)
    : 0;
  const totalWarnings = results
    ? Object.values(results).reduce((sum, r) => sum + r.warnings.length, 0)
    : 0;
  const translationLocales = results
    ? extractLocales(Object.keys(results).filter(p => results[p].file_type === 'translation'))
    : [];

  return (
    <Box flexDirection="column" paddingY={1}>
      {/* Header */}
      <Box marginBottom={1}>
        {phase === 'collecting' && (
          <Box>
            <Text color="cyan"><Spinner type="dots" /></Text>
            <Text> </Text>
            <Gradient colors={['#4ECDC4', '#45B7D1', '#96E6A1']}>
              Collecting files...
            </Gradient>
            <Text dimColor> ({filesCollected} found)</Text>
          </Box>
        )}

        {phase === 'validating' && (
          <Box>
            <Text color="cyan"><Spinner type="dots" /></Text>
            <Text> </Text>
            <Gradient colors={['#FF6B6B', '#4ECDC4', '#45B7D1']}>
              Validating package...
            </Gradient>
          </Box>
        )}

        {phase === 'complete' && totalErrors === 0 && (
          <Box>
            <Text color="green" bold>{ICONS.success} Validation passed!</Text>
          </Box>
        )}

        {phase === 'complete' && totalErrors > 0 && (
          <Box>
            <Text color="red" bold>{ICONS.error} Validation failed</Text>
          </Box>
        )}

        {phase === 'error' && (
          <Box>
            <Text color="red" bold>{ICONS.error} Validation error</Text>
          </Box>
        )}
      </Box>

      {/* Progress bar during validation */}
      {phase === 'validating' && (
        <Box flexDirection="column" marginBottom={1}>
          <ProgressBar progress={progress} />
          {currentFile && (
            <Box marginTop={1}>
              <Text dimColor>  {currentFile}</Text>
            </Box>
          )}
        </Box>
      )}

      {/* File type breakdown */}
      {phase === 'complete' && fileTypeStats && (
        <Box flexDirection="column" marginLeft={2}>
          <Box marginBottom={1}>
            <Text dimColor>{'─'.repeat(35)}</Text>
          </Box>

          {/* Display stats in a nice order */}
          {(['manifest', 'nodetype', 'archetype', 'elementtype', 'workspace', 'content', 'translation'] as FileType[])
            .filter(ft => fileTypeStats.has(ft))
            .map(fileType => (
              <FileTypeRow
                key={fileType}
                fileType={fileType}
                stats={fileTypeStats.get(fileType)!}
              />
            ))
          }

          {translationLocales.length > 0 && (
            <Box marginLeft={4}>
              <Text dimColor>Locales: {translationLocales.join(', ')}</Text>
            </Box>
          )}

          <Box marginTop={1}>
            <Text dimColor>{'─'.repeat(35)}</Text>
          </Box>

          {/* Summary line */}
          <Box marginTop={1}>
            <Text bold>{totalFiles} files</Text>
            <Text> </Text>
            {totalErrors > 0 ? (
              <Text color="red">{totalErrors} error{totalErrors !== 1 ? 's' : ''}</Text>
            ) : (
              <Text color="green">0 errors</Text>
            )}
            <Text> </Text>
            {totalWarnings > 0 ? (
              <Text color="yellow">{totalWarnings} warning{totalWarnings !== 1 ? 's' : ''}</Text>
            ) : (
              <Text dimColor>0 warnings</Text>
            )}
          </Box>
        </Box>
      )}

      {/* Error details with code snippets */}
      {phase === 'complete' && results && totalErrors > 0 && (
        <Box flexDirection="column" marginTop={1}>
          {Object.entries(results)
            .filter(([_, r]) => r.errors.length > 0)
            .flatMap(([filePath, result]) =>
              result.errors.map((err, i) => (
                <ErrorDisplay
                  key={`${filePath}-${i}`}
                  filePath={filePath}
                  error={err}
                  fileContent={fileContents?.[filePath]}
                />
              ))
            )
          }
        </Box>
      )}

      {/* Warning details */}
      {phase === 'complete' && results && totalWarnings > 0 && totalErrors === 0 && (
        <Box flexDirection="column" marginTop={1} marginLeft={2}>
          <Box marginBottom={1}>
            <Text color="yellow">Warnings:</Text>
          </Box>
          {Object.entries(results)
            .filter(([_, r]) => r.warnings.length > 0)
            .map(([filePath, result]) => (
              <Box key={filePath} flexDirection="column" marginBottom={1}>
                <Text dimColor>{filePath}</Text>
                {result.warnings.map((warn, i) => (
                  <Box key={i} marginLeft={2}>
                    <Text color="yellow">{ICONS.warning} </Text>
                    <Text>[{warn.error_code}] {warn.message}</Text>
                  </Box>
                ))}
              </Box>
            ))
          }
        </Box>
      )}

      {/* Error message */}
      {phase === 'error' && error && (
        <Box marginTop={1} marginLeft={2}>
          <Text color="red">{error}</Text>
        </Box>
      )}
    </Box>
  );
}

export default PackageValidator;
