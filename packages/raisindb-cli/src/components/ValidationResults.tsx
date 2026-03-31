/**
 * Component to display validation results for a package
 */

import React from 'react';
import { Box, Text } from 'ink';
import type { PackageValidationResults, ValidationError } from '../wasm/types.js';
import { getValidationSummary } from '../wasm/schema-validator.js';

interface ValidationResultsProps {
  results: PackageValidationResults;
  showWarnings?: boolean;
}

/**
 * Display validation results summary and details
 */
export const ValidationResults: React.FC<ValidationResultsProps> = ({
  results,
  showWarnings = true,
}) => {
  const summary = getValidationSummary(results);

  return (
    <Box flexDirection="column">
      {/* Summary header */}
      <Box marginBottom={1}>
        <Text bold>
          Validation:{' '}
          {summary.hasErrors ? (
            <Text color="red">FAILED</Text>
          ) : (
            <Text color="green">PASSED</Text>
          )}
        </Text>
        <Text dimColor>
          {' '}
          ({summary.errorCount} error{summary.errorCount !== 1 ? 's' : ''},{' '}
          {summary.warningCount} warning{summary.warningCount !== 1 ? 's' : ''})
        </Text>
      </Box>

      {/* File-by-file results */}
      {Object.entries(results).map(([filePath, result]) => {
        const hasIssues =
          result.errors.length > 0 || (showWarnings && result.warnings.length > 0);

        if (!hasIssues) return null;

        return (
          <Box key={filePath} flexDirection="column" marginLeft={2} marginBottom={1}>
            <Text dimColor>{filePath}</Text>

            {/* Errors */}
            {result.errors.map((error, i) => (
              <ErrorLine key={`err-${i}`} error={error} />
            ))}

            {/* Warnings */}
            {showWarnings &&
              result.warnings.map((warning, i) => (
                <WarningLine key={`warn-${i}`} warning={warning} />
              ))}
          </Box>
        );
      })}
    </Box>
  );
};

/**
 * Display a single error line
 */
const ErrorLine: React.FC<{ error: ValidationError }> = ({ error }) => (
  <Box marginLeft={2}>
    <Text color="red">error</Text>
    <Text color="yellow">[{error.error_code}]</Text>
    <Text> {error.message}</Text>
    {error.field_path && (
      <Text dimColor> at {error.field_path}</Text>
    )}
    {error.line && <Text dimColor> (line {error.line})</Text>}
  </Box>
);

/**
 * Display a single warning line
 */
const WarningLine: React.FC<{ warning: ValidationError }> = ({ warning }) => (
  <Box marginLeft={2}>
    <Text color="yellow">warn </Text>
    <Text color="yellow">[{warning.error_code}]</Text>
    <Text> {warning.message}</Text>
    {warning.field_path && (
      <Text dimColor> at {warning.field_path}</Text>
    )}
  </Box>
);

/**
 * Compact summary for inline display
 */
export const ValidationSummary: React.FC<{ results: PackageValidationResults }> = ({
  results,
}) => {
  const summary = getValidationSummary(results);

  if (!summary.hasErrors && summary.warningCount === 0) {
    return (
      <Text color="green">
        All {summary.totalFiles} files validated successfully
      </Text>
    );
  }

  return (
    <Box>
      {summary.hasErrors ? (
        <Text color="red">
          {summary.errorCount} error{summary.errorCount !== 1 ? 's' : ''}
        </Text>
      ) : (
        <Text color="green">No errors</Text>
      )}
      {summary.warningCount > 0 && (
        <Text color="yellow">
          , {summary.warningCount} warning{summary.warningCount !== 1 ? 's' : ''}
        </Text>
      )}
    </Box>
  );
};

export default ValidationResults;
