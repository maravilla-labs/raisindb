import React, { useState } from 'react';
import { Box, Text, useInput } from 'ink';
import TextInput from 'ink-text-input';
import { highlight } from 'sql-highlight';
import ResultsTable from './ResultsTable.js';
import Spinner from './Spinner.js';
import { ErrorMessage } from './StatusMessages.js';
import { executeSql } from '../api.js';

interface SqlShellProps {
  currentDatabase: string | null;
  onExit: () => void;
}

interface QueryResult {
  type: 'success' | 'error';
  query: string;
  data?: { columns: string[]; rows: Record<string, unknown>[] };
  error?: string;
  executionTime?: number;
}

const SqlShell: React.FC<SqlShellProps> = ({ currentDatabase, onExit }) => {
  const [input, setInput] = useState('');
  const [results, setResults] = useState<QueryResult[]>([]);
  const [isExecuting, setIsExecuting] = useState(false);

  // Command history
  const [history, setHistory] = useState<string[]>([]);
  const [historyIndex, setHistoryIndex] = useState(-1);
  const [savedInput, setSavedInput] = useState('');

  // Ctrl+C state for double-press to exit
  const [ctrlCPressed, setCtrlCPressed] = useState(false);

  const prompt = currentDatabase ? `sql:${currentDatabase}> ` : 'sql> ';

  // Handle up/down arrows for history and Ctrl+C
  useInput((inputChar, key) => {
    if (isExecuting) return;

    // Handle Ctrl+C
    if (key.ctrl && inputChar === 'c') {
      if (ctrlCPressed) {
        // Second Ctrl+C - exit SQL mode
        onExit();
      } else {
        // First Ctrl+C - clear input and show hint
        setInput('');
        setCtrlCPressed(true);
        // Reset after 2 seconds
        setTimeout(() => setCtrlCPressed(false), 2000);
      }
      return;
    }

    // Any other key resets Ctrl+C state
    if (ctrlCPressed) {
      setCtrlCPressed(false);
    }

    if (key.upArrow) {
      if (history.length === 0) return;

      if (historyIndex === -1) {
        // First time pressing up, save current input
        setSavedInput(input);
        setHistoryIndex(history.length - 1);
        setInput(history[history.length - 1]);
      } else if (historyIndex > 0) {
        setHistoryIndex(historyIndex - 1);
        setInput(history[historyIndex - 1]);
      }
    } else if (key.downArrow) {
      if (historyIndex === -1) return;

      if (historyIndex < history.length - 1) {
        setHistoryIndex(historyIndex + 1);
        setInput(history[historyIndex + 1]);
      } else {
        // Back to current input
        setHistoryIndex(-1);
        setInput(savedInput);
      }
    }
  });

  const executeQuery = async (query: string) => {
    if (!query.trim()) return;

    if (!currentDatabase) {
      setResults((prev) => [
        ...prev,
        {
          type: 'error',
          query,
          error: 'No database selected. Use "use <database>" command first.',
        },
      ]);
      return;
    }

    setIsExecuting(true);
    const startTime = Date.now();

    try {
      const result = await executeSql(currentDatabase, query);
      const executionTime = result.execution_time_ms || (Date.now() - startTime);

      setResults((prev) => [
        ...prev,
        {
          type: 'success',
          query,
          data: {
            columns: result.columns,
            rows: result.rows as Record<string, unknown>[],
          },
          executionTime,
        },
      ]);
    } catch (error) {
      setResults((prev) => [
        ...prev,
        {
          type: 'error',
          query,
          error: error instanceof Error ? error.message : String(error),
        },
      ]);
    } finally {
      setIsExecuting(false);
    }
  };

  const handleSubmit = async (value: string) => {
    if (!value.trim()) return;

    // Add to history (avoid duplicates)
    if (value.trim() !== history[history.length - 1]) {
      setHistory(prev => [...prev, value.trim()]);
    }
    setHistoryIndex(-1);
    setSavedInput('');

    // Check for exit command (with or without slash)
    if (value.trim() === '/exit' || value.trim() === '/exit-sql' ||
        value.trim() === 'exit' || value.trim() === 'quit') {
      onExit();
      return;
    }

    // Check for clear command
    if (value.trim() === '/clear') {
      setResults([]);
      setInput('');
      return;
    }

    // Execute query
    await executeQuery(value);
    setInput('');
  };

  // Syntax highlight SQL
  const highlightSql = (sql: string): string => {
    try {
      return highlight(sql, { html: false });
    } catch {
      return sql;
    }
  };

  return (
    <Box flexDirection="column">
      {/* Header */}
      <Box marginBottom={1}>
        <Text bold color="#D97706">
          SQL Mode
        </Text>
        <Text dimColor> - Type /exit to return to shell</Text>
      </Box>

      {/* Results */}
      <Box flexDirection="column">
        {results.map((result, idx) => (
          <Box key={idx} flexDirection="column" marginBottom={1}>
            {/* Query */}
            <Box>
              <Text dimColor>Query: </Text>
              <Text color="#EA580C">{result.query}</Text>
            </Box>

            {/* Result */}
            {result.type === 'success' && result.data && (
              <Box flexDirection="column" marginTop={1}>
                <ResultsTable columns={result.data.columns} rows={result.data.rows} />
                <Box marginTop={1}>
                  <Text dimColor>
                    {result.data.rows.length} row(s) in {result.executionTime}ms
                  </Text>
                </Box>
              </Box>
            )}

            {result.type === 'error' && result.error && (
              <Box marginTop={1}>
                <ErrorMessage message={result.error} />
              </Box>
            )}
          </Box>
        ))}
      </Box>

      {/* Spinner during execution */}
      {isExecuting && (
        <Box marginY={1}>
          <Spinner text="Executing query..." />
        </Box>
      )}

      {/* Input prompt */}
      <Box>
        <Text color="#D97706">{prompt}</Text>
        <TextInput value={input} onChange={setInput} onSubmit={handleSubmit} />
      </Box>

      {/* Ctrl+C hint - appears below prompt */}
      {ctrlCPressed && (
        <Box marginTop={1} paddingX={1}>
          <Text color="yellow" dimColor>
            Press Ctrl+C again to exit SQL mode
          </Text>
        </Box>
      )}
    </Box>
  );
};

export default SqlShell;
