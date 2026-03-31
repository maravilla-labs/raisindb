import React from 'react';
import { Box, Text } from 'ink';

interface ResultsTableProps {
  columns: string[];
  rows: Record<string, unknown>[];
  maxColumns?: number;
}

// Priority columns to show first (most useful)
const PRIORITY_COLUMNS = ['id', 'name', 'path', 'node_type', 'properties', 'created_at', 'updated_at'];

// Columns to hide by default (internal/less useful)
const HIDDEN_COLUMNS = ['__revision', '__branch', '__workspace', 'locale', 'version', 'archetype',
  'published_at', 'published_by', 'updated_by', 'created_by', 'translations', 'owner_id', 'relations', 'parent_path'];

const ResultsTable: React.FC<ResultsTableProps> = ({ columns, rows, maxColumns }) => {
  if (rows.length === 0) {
    return (
      <Box>
        <Text dimColor>No results</Text>
      </Box>
    );
  }

  // Get terminal width (default to 120 if not available)
  const terminalWidth = process.stdout.columns || 120;

  // Filter and order columns smartly
  const filterColumns = (cols: string[]): string[] => {
    // Separate priority, normal, and hidden columns
    const priority: string[] = [];
    const normal: string[] = [];
    const hidden: string[] = [];

    for (const col of cols) {
      if (PRIORITY_COLUMNS.includes(col)) {
        priority.push(col);
      } else if (HIDDEN_COLUMNS.includes(col)) {
        hidden.push(col);
      } else {
        normal.push(col);
      }
    }

    // Sort priority columns by their order in PRIORITY_COLUMNS
    priority.sort((a, b) => PRIORITY_COLUMNS.indexOf(a) - PRIORITY_COLUMNS.indexOf(b));

    // Combine: priority first, then normal, hidden last
    const ordered = [...priority, ...normal, ...hidden];

    // Limit to max columns if specified, or fit to terminal
    const limit = maxColumns || Math.min(ordered.length, 8);
    return ordered.slice(0, limit);
  };

  const visibleColumns = filterColumns(columns);
  const hiddenCount = columns.length - visibleColumns.length;

  // Format value for display
  const formatValue = (value: unknown): string => {
    if (value === null || value === undefined) {
      return '';
    }
    if (typeof value === 'object') {
      try {
        const json = JSON.stringify(value);
        // Compact JSON for small objects
        if (json.length <= 30) {
          return json;
        }
        // Count keys for objects
        if (Array.isArray(value)) {
          return `[${value.length} items]`;
        }
        const keys = Object.keys(value as Record<string, unknown>);
        return `{${keys.length} keys}`;
      } catch {
        return '[Object]';
      }
    }
    return String(value);
  };

  // Calculate column widths based on content and available space
  const calculateWidths = (): Record<string, number> => {
    const widths: Record<string, number> = {};
    const separatorWidth = 3; // ' │ '
    const totalSeparators = (visibleColumns.length - 1) * separatorWidth;
    const availableWidth = terminalWidth - totalSeparators - 2;

    // First pass: get natural widths
    for (const col of visibleColumns) {
      const headerWidth = col.length;
      const maxContentWidth = Math.max(
        ...rows.map((row) => formatValue(row[col]).length)
      );
      widths[col] = Math.max(headerWidth, Math.min(maxContentWidth, 40));
    }

    // Second pass: fit to terminal width
    const totalNatural = Object.values(widths).reduce((a, b) => a + b, 0);

    if (totalNatural > availableWidth) {
      // Need to shrink columns
      const ratio = availableWidth / totalNatural;
      for (const col of visibleColumns) {
        widths[col] = Math.max(
          Math.min(col.length, 10), // Minimum width
          Math.floor(widths[col] * ratio)
        );
      }
    }

    return widths;
  };

  const columnWidths = calculateWidths();

  const formatCell = (value: unknown, width: number): string => {
    const str = formatValue(value);
    if (str.length > width) {
      return str.substring(0, width - 1) + '…';
    }
    return str.padEnd(width);
  };

  const separator = visibleColumns
    .map((col) => '─'.repeat(columnWidths[col]))
    .join('─┼─');

  return (
    <Box flexDirection="column">
      {/* Header */}
      <Box>
        <Text bold color="cyan">
          {visibleColumns.map((col, idx) => (
            <React.Fragment key={col}>
              {idx > 0 && <Text color="gray"> │ </Text>}
              <Text bold color="cyan">{col.padEnd(columnWidths[col]).substring(0, columnWidths[col])}</Text>
            </React.Fragment>
          ))}
        </Text>
      </Box>

      {/* Separator */}
      <Box>
        <Text color="gray">{separator}</Text>
      </Box>

      {/* Rows */}
      {rows.map((row, rowIdx) => (
        <Box key={rowIdx}>
          <Text>
            {visibleColumns.map((col, colIdx) => (
              <React.Fragment key={col}>
                {colIdx > 0 && <Text color="gray"> │ </Text>}
                <Text>{formatCell(row[col], columnWidths[col])}</Text>
              </React.Fragment>
            ))}
          </Text>
        </Box>
      ))}

      {/* Footer */}
      <Box marginTop={1} flexDirection="column">
        <Text dimColor>
          {rows.length} row{rows.length !== 1 ? 's' : ''} returned
          {hiddenCount > 0 && ` (${hiddenCount} columns hidden)`}
        </Text>
      </Box>
    </Box>
  );
};

export default ResultsTable;
