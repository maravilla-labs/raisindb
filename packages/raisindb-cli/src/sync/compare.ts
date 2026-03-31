/**
 * File comparison utilities for package sync
 */

import fs from 'fs';
import path from 'path';
import crypto from 'crypto';
import { SyncConfig, shouldIgnore } from './config.js';

/**
 * Status of a file in sync comparison
 */
export type SyncFileStatus =
  | 'synced'
  | 'modified'
  | 'local_only'
  | 'server_only'
  | 'conflict';

/**
 * Information about a file change
 */
export interface FileChange {
  path: string;
  status: SyncFileStatus;
  localHash?: string;
  serverHash?: string;
  localMtime?: Date;
  serverMtime?: Date;
  localContent?: string;
  serverContent?: string;
}

/**
 * Result of comparing local and server state
 */
export interface SyncDiff {
  toUpload: FileChange[];
  toDownload: FileChange[];
  conflicts: FileChange[];
  unchanged: string[];
}

/**
 * Server file info returned by API
 */
export interface ServerFileInfo {
  path: string;
  hash: string;
  modified_at: string;
  node_type?: string;
}

/**
 * Compute SHA-256 hash of file content
 */
export function computeHash(content: Buffer | string): string {
  const hash = crypto.createHash('sha256');
  hash.update(content);
  return `sha256:${hash.digest('hex')}`;
}

/**
 * Compute hash of a file
 */
export function computeFileHash(filePath: string): string {
  const content = fs.readFileSync(filePath);
  return computeHash(content);
}

/**
 * Get all files in a directory recursively
 * For sync purposes, only scans the content/ folder if it exists
 */
export function getLocalFiles(
  directory: string,
  config: SyncConfig,
  basePath: string = ''
): Map<string, { hash: string; mtime: Date }> {
  const files = new Map<string, { hash: string; mtime: Date }>();

  // If this is the root call (basePath is empty), look for content/ folder
  let scanDir = directory;
  if (basePath === '') {
    const contentDir = path.join(directory, 'content');
    if (fs.existsSync(contentDir) && fs.statSync(contentDir).isDirectory()) {
      scanDir = contentDir;
    }
  }

  const entries = fs.readdirSync(scanDir, { withFileTypes: true });

  for (const entry of entries) {
    const relativePath = basePath ? `${basePath}/${entry.name}` : entry.name;
    const fullPath = path.join(scanDir, entry.name);

    // Skip ignored paths
    if (shouldIgnore(config, relativePath)) {
      continue;
    }

    if (entry.isDirectory()) {
      // Recurse into subdirectory
      const subFiles = getLocalFilesRecursive(fullPath, config, relativePath);
      for (const [subPath, info] of subFiles) {
        files.set(subPath, info);
      }
    } else if (entry.isFile()) {
      const stat = fs.statSync(fullPath);
      const hash = computeFileHash(fullPath);

      files.set(relativePath, {
        hash,
        mtime: stat.mtime,
      });
    }
  }

  return files;
}

/**
 * Helper function to recursively get files (used after entering content/ folder)
 */
function getLocalFilesRecursive(
  directory: string,
  config: SyncConfig,
  basePath: string
): Map<string, { hash: string; mtime: Date }> {
  const files = new Map<string, { hash: string; mtime: Date }>();
  const entries = fs.readdirSync(directory, { withFileTypes: true });

  for (const entry of entries) {
    const relativePath = `${basePath}/${entry.name}`;
    const fullPath = path.join(directory, entry.name);

    // Skip ignored paths
    if (shouldIgnore(config, relativePath)) {
      continue;
    }

    if (entry.isDirectory()) {
      // Recurse into subdirectory
      const subFiles = getLocalFilesRecursive(fullPath, config, relativePath);
      for (const [subPath, info] of subFiles) {
        files.set(subPath, info);
      }
    } else if (entry.isFile()) {
      const stat = fs.statSync(fullPath);
      const hash = computeFileHash(fullPath);

      files.set(relativePath, {
        hash,
        mtime: stat.mtime,
      });
    }
  }

  return files;
}

/**
 * Compare local files with server files
 */
export function compareFiles(
  localFiles: Map<string, { hash: string; mtime: Date }>,
  serverFiles: Map<string, ServerFileInfo>,
  lastSyncHash?: string
): SyncDiff {
  const toUpload: FileChange[] = [];
  const toDownload: FileChange[] = [];
  const conflicts: FileChange[] = [];
  const unchanged: string[] = [];

  const allPaths = new Set([...localFiles.keys(), ...serverFiles.keys()]);

  for (const filePath of allPaths) {
    const local = localFiles.get(filePath);
    const server = serverFiles.get(filePath);

    if (local && server) {
      // File exists in both places
      if (local.hash === server.hash) {
        unchanged.push(filePath);
      } else {
        // Hashes differ - determine conflict or one-way change
        // For now, we consider any difference a conflict that needs resolution
        conflicts.push({
          path: filePath,
          status: 'conflict',
          localHash: local.hash,
          serverHash: server.hash,
          localMtime: local.mtime,
          serverMtime: new Date(server.modified_at),
        });
      }
    } else if (local && !server) {
      // File exists only locally
      toUpload.push({
        path: filePath,
        status: 'local_only',
        localHash: local.hash,
        localMtime: local.mtime,
      });
    } else if (!local && server) {
      // File exists only on server
      toDownload.push({
        path: filePath,
        status: 'server_only',
        serverHash: server.hash,
        serverMtime: new Date(server.modified_at),
      });
    }
  }

  return { toUpload, toDownload, conflicts, unchanged };
}

/**
 * Generate unified diff between two strings
 */
export function generateUnifiedDiff(
  localContent: string,
  serverContent: string,
  filename: string
): string {
  const localLines = localContent.split('\n');
  const serverLines = serverContent.split('\n');

  const lines: string[] = [];
  lines.push(`--- local/${filename}`);
  lines.push(`+++ server/${filename}`);

  // Simple line-by-line diff (a proper implementation would use LCS algorithm)
  let i = 0, j = 0;
  while (i < localLines.length || j < serverLines.length) {
    if (i >= localLines.length) {
      lines.push(`+${serverLines[j]}`);
      j++;
    } else if (j >= serverLines.length) {
      lines.push(`-${localLines[i]}`);
      i++;
    } else if (localLines[i] === serverLines[j]) {
      lines.push(` ${localLines[i]}`);
      i++;
      j++;
    } else {
      // Simple approach: show removed then added
      lines.push(`-${localLines[i]}`);
      lines.push(`+${serverLines[j]}`);
      i++;
      j++;
    }
  }

  return lines.join('\n');
}

/**
 * Summary of sync diff
 */
export interface SyncSummary {
  toUpload: number;
  toDownload: number;
  conflicts: number;
  unchanged: number;
  total: number;
}

/**
 * Get summary of sync diff
 */
export function getSyncSummary(diff: SyncDiff): SyncSummary {
  return {
    toUpload: diff.toUpload.length,
    toDownload: diff.toDownload.length,
    conflicts: diff.conflicts.length,
    unchanged: diff.unchanged.length,
    total: diff.toUpload.length + diff.toDownload.length + diff.conflicts.length + diff.unchanged.length,
  };
}
