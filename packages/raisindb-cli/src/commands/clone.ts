/**
 * Package clone command - download a package from server to local directory
 */

import fs from 'fs';
import path from 'path';
import React, { useState, useEffect } from 'react';
import { render } from 'ink';
import extractZip from 'extract-zip';
import { getServer, loadConfig } from '../config.js';
import { getToken } from '../auth.js';
import {
  listPackages,
  exportPackage,
  downloadExportedPackage,
  subscribeToJobEvents,
  type JobEvent,
} from '../api.js';
import { PackageSelector } from '../components/PackageSelector.js';
import { CloneProgress, type ClonePhase } from '../components/CloneProgress.js';

/**
 * Clone command options
 */
export interface CloneOptions {
  output?: string;
  server?: string;
  repo?: string;
  branch?: string;
}

/**
 * Main clone function
 */
export async function clonePackage(
  name?: string,
  options: CloneOptions = {}
): Promise<void> {
  // Get config
  const config = loadConfig();
  const repo = options.repo || config.default_repo || 'default';
  const branch = options.branch || 'main';

  // Verify authentication
  const token = getToken();
  if (!token) {
    throw new Error('Not authenticated. Run "raisindb shell" and use /login first.');
  }

  // If no package name provided, show interactive selector
  if (!name) {
    await runInteractiveClone(repo, branch, options);
    return;
  }

  // Clone the specified package
  await runClone(name, repo, branch, options);
}

/**
 * Run interactive package selection and clone
 */
async function runInteractiveClone(
  repo: string,
  branch: string,
  options: CloneOptions
): Promise<void> {
  return new Promise((resolve, reject) => {
    const { unmount } = render(
      React.createElement(InteractiveClone, {
        repo,
        branch,
        options,
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

/**
 * Interactive clone component
 */
function InteractiveClone({
  repo,
  branch,
  options,
  onComplete,
  onError,
}: {
  repo: string;
  branch: string;
  options: CloneOptions;
  onComplete: () => void;
  onError: (error: Error) => void;
}) {
  const [selectedPackage, setSelectedPackage] = useState<string | null>(null);

  if (!selectedPackage) {
    return React.createElement(PackageSelector, {
      repo,
      onSelect: (pkg: string) => setSelectedPackage(pkg),
      onCancel: () => {
        console.log('Clone cancelled.');
        onComplete();
      },
    });
  }

  return React.createElement(CloneRunner, {
    packageName: selectedPackage,
    repo,
    branch,
    options,
    onComplete,
    onError,
  });
}

/**
 * Clone runner component that shows progress
 */
function CloneRunner({
  packageName,
  repo,
  branch,
  options,
  onComplete,
  onError,
}: {
  packageName: string;
  repo: string;
  branch: string;
  options: CloneOptions;
  onComplete: () => void;
  onError: (error: Error) => void;
}) {
  const [phase, setPhase] = useState<ClonePhase>('preparing');
  const [progress, setProgress] = useState(0);
  const [message, setMessage] = useState<string | undefined>();
  const [error, setError] = useState<string | undefined>();
  const [outputDir, setOutputDir] = useState<string | undefined>();

  useEffect(() => {
    runCloneProcess();
  }, []);

  async function runCloneProcess() {
    try {
      // Determine output directory
      const targetDir = options.output || path.join(process.cwd(), packageName);
      setOutputDir(targetDir);

      // Check if directory already exists
      if (fs.existsSync(targetDir)) {
        const stats = fs.statSync(targetDir);
        if (stats.isDirectory()) {
          const files = fs.readdirSync(targetDir);
          if (files.length > 0) {
            throw new Error(`Directory "${targetDir}" already exists and is not empty`);
          }
        } else {
          throw new Error(`"${targetDir}" exists and is not a directory`);
        }
      }

      // Start export
      setPhase('exporting');
      setMessage('Starting package export...');
      setProgress(10);

      const exportResult = await exportPackage(repo, packageName, {
        export_mode: 'all',
        include_modifications: true,
      }, branch);

      const jobId = exportResult.job_id;

      // Wait for export job to complete
      await waitForJob(jobId, (jobProgress) => {
        setProgress(10 + Math.round(jobProgress * 40));
        setMessage(`Exporting: ${Math.round(jobProgress * 100)}%`);
      });

      setProgress(50);
      setPhase('downloading');
      setMessage('Downloading package...');

      // Download the exported package
      const packageData = await downloadExportedPackage(repo, packageName, jobId, branch);
      setProgress(70);

      // Create temp file for extraction
      const tempFile = path.join(process.cwd(), `.${packageName}-${Date.now()}.rap`);
      fs.writeFileSync(tempFile, Buffer.from(packageData));

      setPhase('extracting');
      setMessage('Extracting files...');
      setProgress(80);

      // Create output directory
      fs.mkdirSync(targetDir, { recursive: true });

      // Extract the package
      await extractZip(tempFile, { dir: targetDir });

      // Clean up temp file
      fs.unlinkSync(tempFile);

      setProgress(100);
      setPhase('complete');
      setMessage(undefined);

      // Give time to show completion message
      setTimeout(onComplete, 1000);
    } catch (err) {
      setPhase('error');
      const errorMsg = err instanceof Error ? err.message : String(err);
      setError(errorMsg);
      onError(new Error(errorMsg));
    }
  }

  return React.createElement(CloneProgress, {
    phase,
    packageName,
    progress,
    message,
    error,
    outputDir,
  });
}

/**
 * Wait for a job to complete
 */
function waitForJob(
  jobId: string,
  onProgress?: (progress: number) => void
): Promise<void> {
  return new Promise((resolve, reject) => {
    let completed = false;

    const cleanup = subscribeToJobEvents(
      (event: JobEvent) => {
        if (event.job_id !== jobId) return;

        if (event.progress !== null && onProgress) {
          onProgress(event.progress);
        }

        if (event.status === 'Completed') {
          completed = true;
          cleanup();
          resolve();
        } else if (event.status.startsWith('Failed')) {
          completed = true;
          cleanup();
          reject(new Error(event.error || 'Job failed'));
        }
      },
      (error) => {
        if (!completed) {
          cleanup();
          reject(error);
        }
      }
    );

    // Timeout after 5 minutes
    setTimeout(() => {
      if (!completed) {
        cleanup();
        reject(new Error('Export job timed out'));
      }
    }, 5 * 60 * 1000);
  });
}

/**
 * Run clone directly (non-interactive)
 */
async function runClone(
  packageName: string,
  repo: string,
  branch: string,
  options: CloneOptions
): Promise<void> {
  return new Promise((resolve, reject) => {
    const { unmount } = render(
      React.createElement(CloneRunner, {
        packageName,
        repo,
        branch,
        options,
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
