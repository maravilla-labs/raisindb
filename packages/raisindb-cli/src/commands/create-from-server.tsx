/**
 * Package create-from-server command - create a package by selecting content from server
 */

import fs from 'fs';
import path from 'path';
import React, { useState, useEffect } from 'react';
import { render, Box, Text, useInput, useApp } from 'ink';
import TextInput from 'ink-text-input';
import Spinner from 'ink-spinner';
import Gradient from 'ink-gradient';
import { getToken } from '../auth.js';
import { loadConfig } from '../config.js';
import {
  createPackageFromSelection,
  subscribeToJobEvents,
  getBaseUrl,
  getHeaders,
  type SelectedPath,
  type JobEvent,
} from '../api.js';
import TreeSelector, { type SelectedNode } from '../components/TreeSelector.js';

export interface CreateFromServerOptions {
  server?: string;
  repo?: string;
}

type Phase = 'selection' | 'metadata' | 'creating' | 'complete' | 'error';

/**
 * Main create-from-server function
 */
export async function createFromServer(options: CreateFromServerOptions = {}): Promise<void> {
  // Get config
  const config = loadConfig();
  const repo = options.repo || config.default_repo || 'default';

  // Verify authentication
  const token = getToken();
  if (!token) {
    throw new Error('Not authenticated. Run "raisindb shell" and use /login first.');
  }

  return new Promise((resolve, reject) => {
    const { unmount } = render(
      React.createElement(CreateFromServerFlow, {
        repo,
        onComplete: () => {
          unmount();
          resolve();
        },
        onError: (error: Error) => {
          unmount();
          reject(error);
        },
      })
    );
  });
}

interface CreateFromServerFlowProps {
  repo: string;
  onComplete: () => void;
  onError: (error: Error) => void;
}

function CreateFromServerFlow({ repo, onComplete, onError }: CreateFromServerFlowProps) {
  const { exit } = useApp();

  const [phase, setPhase] = useState<Phase>('selection');
  const [selectedNodes, setSelectedNodes] = useState<SelectedNode[]>([]);

  // Metadata form state
  const [packageName, setPackageName] = useState('');
  const [packageVersion, setPackageVersion] = useState('1.0.0');
  const [focusField, setFocusField] = useState<'name' | 'version'>('name');

  // Progress state
  const [progress, setProgress] = useState(0);
  const [statusMessage, setStatusMessage] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [downloadPath, setDownloadPath] = useState<string | null>(null);

  // Handle selection complete
  const handleSelectionComplete = (selected: SelectedNode[]) => {
    setSelectedNodes(selected);
    setPhase('metadata');
  };

  // Handle cancel
  const handleCancel = () => {
    onComplete();
  };

  // Handle metadata submission
  const handleMetadataSubmit = async () => {
    if (!packageName.trim()) {
      setError('Package name is required');
      return;
    }

    setPhase('creating');
    setStatusMessage('Starting package creation...');
    setProgress(10);

    try {
      const paths: SelectedPath[] = selectedNodes.map(n => ({
        workspace: n.workspace,
        path: n.isRecursive ? `${n.path}/*` : n.path,
      }));

      const response = await createPackageFromSelection(repo, {
        name: packageName.trim(),
        version: packageVersion.trim() || '1.0.0',
        selected_paths: paths,
        include_node_types: true,
      });

      const serverDownloadPath = response.download_path;

      // Helper function to download the package after job completes
      const downloadPackageFile = async () => {
        try {
          setProgress(92);
          setStatusMessage('Downloading package...');

          const baseUrl = getBaseUrl();
          const downloadUrl = `${baseUrl}${serverDownloadPath}`;

          const downloadResponse = await fetch(downloadUrl, {
            method: 'GET',
            headers: {
              Authorization: getHeaders().Authorization || '',
            },
          });

          if (!downloadResponse.ok) {
            throw new Error(`Download failed: ${downloadResponse.status}`);
          }

          setProgress(96);
          setStatusMessage('Saving package...');

          const buffer = await downloadResponse.arrayBuffer();
          const version = packageVersion.trim() || '1.0.0';
          const filename = `${packageName.trim()}-${version}.rap`;
          const outputPath = path.join(process.cwd(), filename);

          fs.writeFileSync(outputPath, Buffer.from(buffer));
          setDownloadPath(outputPath);
          setProgress(100);
          setPhase('complete');
          setStatusMessage('Package created and downloaded!');
        } catch (downloadErr) {
          // If download fails, still show success with server URL
          setDownloadPath(serverDownloadPath);
          setProgress(100);
          setPhase('complete');
          setStatusMessage('Package created (download failed, use URL)');
        }
      };

      // Subscribe to job events
      const cleanup = subscribeToJobEvents(
        (event: JobEvent) => {
          if (event.job_id !== response.job_id) return;

          if (event.progress !== null) {
            setProgress(10 + Math.round(event.progress * 80));
          }

          if (event.status === 'Completed') {
            cleanup();
            // Download the package after job completes
            downloadPackageFile();
          } else if (event.status.startsWith('Failed')) {
            setPhase('error');
            setError(event.error || 'Package creation failed');
            cleanup();
          }
        },
        (err) => {
          setPhase('error');
          setError(err.message);
        }
      );

      // Timeout after 5 minutes
      setTimeout(() => {
        if (phase === 'creating') {
          cleanup();
          setPhase('error');
          setError('Package creation timed out');
        }
      }, 5 * 60 * 1000);
    } catch (err) {
      setPhase('error');
      setError(err instanceof Error ? err.message : 'Unknown error');
    }
  };

  // Handle input in metadata phase
  useInput((input, key) => {
    if (phase !== 'metadata') return;

    if (key.escape) {
      setPhase('selection');
      return;
    }

    if (key.tab) {
      setFocusField(prev => prev === 'name' ? 'version' : 'name');
      return;
    }

    if (key.return) {
      handleMetadataSubmit();
      return;
    }
  }, { isActive: phase === 'metadata' });

  // Handle input in complete phase
  useInput((input, key) => {
    if (phase !== 'complete' && phase !== 'error') return;

    if (key.return || key.escape || input === 'q') {
      onComplete();
    }
  }, { isActive: phase === 'complete' || phase === 'error' });

  // Render selection phase
  if (phase === 'selection') {
    return React.createElement(TreeSelector, {
      repo,
      onComplete: handleSelectionComplete,
      onCancel: handleCancel,
    });
  }

  // Render metadata form
  if (phase === 'metadata') {
    return (
      <Box flexDirection="column" paddingY={1}>
        <Box marginBottom={1}>
          <Text bold>
            <Gradient colors={['#4ECDC4', '#45B7D1']}>
              Package Details
            </Gradient>
          </Text>
          <Text color="gray"> ({selectedNodes.length} items selected)</Text>
        </Box>

        <Box flexDirection="column" borderStyle="round" paddingX={2} paddingY={1}>
          <Box marginBottom={1}>
            <Text color={focusField === 'name' ? 'cyan' : 'white'}>Package Name: </Text>
            <TextInput
              value={packageName}
              onChange={setPackageName}
              focus={focusField === 'name'}
              placeholder="my-package"
            />
          </Box>

          <Box>
            <Text color={focusField === 'version' ? 'cyan' : 'white'}>Version: </Text>
            <TextInput
              value={packageVersion}
              onChange={setPackageVersion}
              focus={focusField === 'version'}
              placeholder="1.0.0"
            />
          </Box>
        </Box>

        {error && (
          <Box marginTop={1}>
            <Text color="red">{error}</Text>
          </Box>
        )}

        <Box marginTop={1}>
          <Text color="gray">Tab: switch field | Enter: create | Esc: back to selection</Text>
        </Box>
      </Box>
    );
  }

  // Render creating phase
  if (phase === 'creating') {
    const barWidth = 30;
    const filledWidth = Math.round((progress / 100) * barWidth);
    const progressBar = '\u2588'.repeat(filledWidth) + '\u2591'.repeat(barWidth - filledWidth);

    return (
      <Box flexDirection="column" paddingY={1}>
        <Box marginBottom={1}>
          <Text color="cyan"><Spinner type="dots" /> </Text>
          <Gradient colors={['#4ECDC4', '#45B7D1', '#96E6A1']}>
            Creating Package
          </Gradient>
        </Box>

        <Box>
          <Text color="cyan">[</Text>
          <Gradient colors={['#4ECDC4', '#45B7D1', '#96E6A1']}>
            {progressBar}
          </Gradient>
          <Text color="cyan">]</Text>
          <Text> {progress}%</Text>
        </Box>

        <Box marginTop={1}>
          <Text color="gray">{statusMessage}</Text>
        </Box>
      </Box>
    );
  }

  // Render complete phase
  if (phase === 'complete') {
    return (
      <Box flexDirection="column" paddingY={1}>
        <Box marginBottom={1}>
          <Text color="green" bold>{'\u2714'} Package Created Successfully</Text>
        </Box>

        <Box flexDirection="column">
          <Box>
            <Text color="gray">Package: </Text>
            <Text color="white" bold>{packageName}</Text>
          </Box>
          <Box>
            <Text color="gray">Version: </Text>
            <Text color="white">{packageVersion}</Text>
          </Box>
          {downloadPath && (
            <Box marginTop={1}>
              <Text color="gray">{downloadPath.startsWith('/') ? 'Download URL: ' : 'Saved to: '}</Text>
              <Text color="cyan">{downloadPath}</Text>
            </Box>
          )}
        </Box>

        <Box marginTop={1}>
          <Text color="gray">Press Enter or Esc to exit</Text>
        </Box>
      </Box>
    );
  }

  // Render error phase
  if (phase === 'error') {
    return (
      <Box flexDirection="column" paddingY={1}>
        <Box marginBottom={1}>
          <Text color="red" bold>{'\u2718'} Package Creation Failed</Text>
        </Box>

        <Box>
          <Text color="red">{error}</Text>
        </Box>

        <Box marginTop={1}>
          <Text color="gray">Press Enter or Esc to exit</Text>
        </Box>
      </Box>
    );
  }

  return null;
}

export default createFromServer;
