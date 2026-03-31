/**
 * Watch mode UI component
 * Shows connection status, activity log, and pending changes
 */

import React, { useState, useEffect } from 'react';
import { Box, Text, useInput, useApp } from 'ink';
import Spinner from 'ink-spinner';
import Gradient from 'ink-gradient';
import { SyncWatcher, WatcherStatus, ChangeEvent } from '../sync/watcher.js';
import { SyncResult } from '../sync/operations.js';

interface ActivityLogEntry {
  id: number;
  timestamp: number;
  type: 'local' | 'server' | 'sync' | 'error' | 'info';
  message: string;
  path?: string;
  details?: string;
}

interface WatchModeProps {
  watcher: SyncWatcher;
  packageDir: string;
  remotePath: string;
  serverUrl: string;
  onExit: () => void;
}

export function WatchMode({
  watcher,
  packageDir,
  remotePath,
  serverUrl,
  onExit,
}: WatchModeProps) {
  const { exit } = useApp();
  const [status, setStatus] = useState<WatcherStatus>(watcher.getStatus());
  const [activityLog, setActivityLog] = useState<ActivityLogEntry[]>([]);
  const [verbose, setVerbose] = useState(false);
  const logIdRef = React.useRef(0);

  // Add entry to activity log
  const addLogEntry = (
    type: ActivityLogEntry['type'],
    message: string,
    path?: string,
    details?: string
  ) => {
    const id = logIdRef.current++;
    setActivityLog((prev) => {
      const newEntry: ActivityLogEntry = {
        id,
        timestamp: Date.now(),
        type,
        message,
        path,
        details,
      };
      // Keep last 20 entries
      return [...prev.slice(-19), newEntry];
    });
  };

  // Handle watcher events
  useEffect(() => {
    const handleStatus = (newStatus: WatcherStatus) => {
      setStatus(newStatus);
    };

    const handleLocalChange = (event: ChangeEvent) => {
      const action =
        event.type === 'add'
          ? 'created'
          : event.type === 'change'
          ? 'modified'
          : event.type === 'unlink'
          ? 'deleted'
          : event.type;
      addLogEntry('local', `Local file ${action}`, event.path);
    };

    const handleServerChange = (event: ChangeEvent) => {
      const action =
        event.type === 'add'
          ? 'created'
          : event.type === 'change'
          ? 'updated'
          : event.type === 'unlink'
          ? 'deleted'
          : event.type;
      addLogEntry('server', `Server node ${action}`, event.path);
    };

    const handleBatch = (batch: {
      localChanges: ChangeEvent[];
      serverChanges: ChangeEvent[];
      conflicts: Array<{ local: ChangeEvent; server: ChangeEvent }>;
    }) => {
      if (batch.localChanges.length > 0) {
        addLogEntry('sync', `Pushing ${batch.localChanges.length} local changes...`);
      }
      if (batch.serverChanges.length > 0) {
        addLogEntry('sync', `Pulling ${batch.serverChanges.length} server changes...`);
      }
      if (batch.conflicts.length > 0) {
        addLogEntry('error', `${batch.conflicts.length} conflict(s) detected`);
        for (const conflict of batch.conflicts) {
          addLogEntry('error', `Conflict: ${conflict.local.path}`);
        }
      }
    };

    const handleError = (error: Error) => {
      addLogEntry('error', error.message);
    };

    const handleLocalReady = () => {
      addLogEntry('info', 'Local file watcher ready');
    };

    const handleServerConnected = () => {
      addLogEntry('info', 'Connected to server');
    };

    const handleServerSubscribed = () => {
      addLogEntry('info', 'Subscribed to server events');
    };

    const handleStopped = () => {
      addLogEntry('info', 'Watch mode stopped');
    };

    const handleSyncResult = (result: SyncResult) => {
      if (result.success) {
        const verb = result.operation === 'push' ? 'Pushed' : 'Pulled';
        addLogEntry('sync', `${verb} successfully`, result.path);
      } else {
        const verb = result.operation === 'push' ? 'push' : 'pull';
        addLogEntry('error', `Failed to ${verb}: ${result.error}`, result.path, result.details);
      }
    };

    watcher.on('status', handleStatus);
    watcher.on('localChange', handleLocalChange);
    watcher.on('serverChange', handleServerChange);
    watcher.on('batch', handleBatch);
    watcher.on('error', handleError);
    watcher.on('localReady', handleLocalReady);
    watcher.on('serverConnected', handleServerConnected);
    watcher.on('serverSubscribed', handleServerSubscribed);
    watcher.on('stopped', handleStopped);
    watcher.on('syncResult', handleSyncResult);

    return () => {
      watcher.off('status', handleStatus);
      watcher.off('localChange', handleLocalChange);
      watcher.off('serverChange', handleServerChange);
      watcher.off('batch', handleBatch);
      watcher.off('error', handleError);
      watcher.off('localReady', handleLocalReady);
      watcher.off('serverConnected', handleServerConnected);
      watcher.off('serverSubscribed', handleServerSubscribed);
      watcher.off('stopped', handleStopped);
      watcher.off('syncResult', handleSyncResult);
    };
  }, [watcher]);

  // Handle keyboard input
  useInput((input, key) => {
    if (key.escape || input === 'q' || (key.ctrl && input === 'c')) {
      onExit();
    }
    if (input === 'v') {
      setVerbose((v) => !v);
    }
  });

  // Status indicator component
  const StatusIndicator = ({
    active,
    label,
  }: {
    active: boolean;
    label: string;
  }) => (
    <Box>
      <Text color={active ? 'green' : 'red'}>{active ? '●' : '○'}</Text>
      <Text color={active ? 'white' : 'gray'}> {label}</Text>
    </Box>
  );

  // Format timestamp
  const formatTime = (timestamp: number) => {
    const date = new Date(timestamp);
    return date.toLocaleTimeString('en-US', {
      hour12: false,
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  };

  // Get log entry color
  const getLogColor = (type: ActivityLogEntry['type']) => {
    switch (type) {
      case 'local':
        return 'cyan';
      case 'server':
        return 'magenta';
      case 'sync':
        return 'yellow';
      case 'error':
        return 'red';
      case 'info':
        return 'gray';
      default:
        return 'white';
    }
  };

  // Get log entry prefix
  const getLogPrefix = (type: ActivityLogEntry['type']) => {
    switch (type) {
      case 'local':
        return '[LOCAL]';
      case 'server':
        return '[SERVER]';
      case 'sync':
        return '[SYNC]';
      case 'error':
        return '[ERROR]';
      case 'info':
        return '[INFO]';
      default:
        return '';
    }
  };

  return (
    <Box flexDirection="column" paddingY={1}>
      {/* Header */}
      <Box marginBottom={1}>
        <Text>
          <Spinner type="dots" />{' '}
        </Text>
        <Gradient colors={['#4ECDC4', '#45B7D1', '#96E6A1']}>
          Watch Mode Active
        </Gradient>
      </Box>

      {/* Connection info */}
      <Box
        flexDirection="column"
        borderStyle="round"
        borderColor="gray"
        paddingX={2}
        paddingY={1}
        marginBottom={1}
      >
        <Box>
          <Text color="gray">Local: </Text>
          <Text color="white">{packageDir}</Text>
        </Box>
        <Box>
          <Text color="gray">Remote: </Text>
          <Text color="cyan">{serverUrl}{remotePath}</Text>
        </Box>
      </Box>

      {/* Status indicators */}
      <Box marginBottom={1}>
        <StatusIndicator active={status.localWatching} label="Local" />
        <Text> </Text>
        <StatusIndicator active={status.connected} label="Server" />
        <Text> </Text>
        <StatusIndicator active={status.serverSubscribed} label="Events" />
        {status.pendingChanges > 0 && (
          <Box marginLeft={2}>
            <Text color="yellow">
              <Spinner type="dots" /> {status.pendingChanges} pending
            </Text>
          </Box>
        )}
      </Box>

      {/* Activity log */}
      <Box
        flexDirection="column"
        borderStyle="round"
        borderColor="gray"
        paddingX={2}
        paddingY={1}
        height={15}
      >
        <Text bold color="gray">
          Activity Log
        </Text>
        <Box flexDirection="column" marginTop={1}>
          {activityLog.length === 0 ? (
            <Text color="gray">Waiting for changes...</Text>
          ) : (
            activityLog.slice(-12).map((entry) => (
              <Box key={entry.id} flexDirection="column">
                <Box>
                  <Text color="gray">{formatTime(entry.timestamp)} </Text>
                  <Text color={getLogColor(entry.type)}>
                    {getLogPrefix(entry.type)}{' '}
                  </Text>
                  <Text color="white">{entry.message}</Text>
                  {entry.path && <Text color="gray"> {entry.path}</Text>}
                </Box>
                {verbose && entry.details && (
                  <Box marginLeft={2} flexDirection="column">
                    {entry.details.split('\n').map((line, i) => (
                      <Text key={i} color="gray" dimColor>{line}</Text>
                    ))}
                  </Box>
                )}
              </Box>
            ))
          )}
        </Box>
      </Box>

      {/* Help */}
      <Box marginTop={1}>
        <Text color="gray">Press </Text>
        <Text color="cyan">v</Text>
        <Text color="gray"> verbose{verbose ? ' (on)' : ''} </Text>
        <Text color="gray"> </Text>
        <Text color="cyan">q</Text>
        <Text color="gray">/</Text>
        <Text color="cyan">Esc</Text>
        <Text color="gray"> quit</Text>
      </Box>
    </Box>
  );
}

export default WatchMode;
