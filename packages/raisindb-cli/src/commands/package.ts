import fs from 'fs';
import path from 'path';
import yaml from 'yaml';
import AdmZip from 'adm-zip';
import ignore, { Ignore } from 'ignore';
import React from 'react';
import { render } from 'ink';
import { getServer, getDefaultRepo } from '../config.js';
import { getToken } from '../auth.js';
import {
  type PackageInstallMode,
  uploadPackage as apiUploadPackage,
  listPackages as apiListPackages,
  installPackage as apiInstallPackage,
  getJobInfo as apiGetJobInfo,
  subscribeToJobEvents,
  PackageSummary,
  JobEvent,
} from '../api.js';
import {
  validatePackageDirectory,
  getValidationSummary,
  applyFix,
  collectPackageFiles,
  initSchemaValidator,
} from '../wasm/schema-validator.js';
import type { PackageValidationResults, ValidationError } from '../wasm/types.js';
import { UploadProgress } from '../components/UploadProgress.js';
import { PackageValidator } from '../components/PackageValidator.js';
import {
  EnvContext,
  UnresolvedToken,
  emptyEnvContext,
  formatUnresolvedError,
  hasEnvTokens,
  substituteEnvTokens,
} from '../env/substitute.js';
import { assertEnvFilesExist, isSubstitutableFile, loadEnvContext } from '../env/load.js';

/**
 * Default patterns that are always ignored when creating packages
 */
export const DEFAULT_IGNORE_PATTERNS = [
  // Version control
  '.git',
  '.git/**',
  '.svn',
  '.svn/**',
  '.hg',
  '.hg/**',

  // OS files
  '.DS_Store',
  'Thumbs.db',
  'desktop.ini',

  // Editor/IDE
  '.idea',
  '.idea/**',
  '.vscode',
  '.vscode/**',
  '*.swp',
  '*.swo',
  '*~',

  // Dependencies (these shouldn't be in package plugins anyway)
  'node_modules',
  'node_modules/**',

  // Build artifacts
  '*.log',
  '*.tmp',
  '*.temp',

  // Guest-toolchain build output. A wasm function's `target/` holds hundreds of
  // MB of cargo intermediates next to the one artifact that ships; the artifact
  // itself lives under content/, not here.
  'target',
  'target/**',

  // Environment files feeding {env:...} substitution. Their VALUES are baked
  // into the YAML at pack time; the files themselves must never ship.
  '.env',
  '.env.*',
];

interface PackageManifest {
  name: string;
  version: string;
  description?: string;
  author?: string;
  dependencies?: Record<string, string>;
  files?: string[];
}

export interface CreatePackageOptions {
  noValidate?: boolean;
  validateOnly?: boolean;
  /** Profile for {env:...} substitution: --env production → .env.production */
  env?: string;
  /** Extra env files from --env-file, applied after the conventional ones. */
  envFile?: string[];
}

/**
 * State for validation progress display
 */
interface ValidationState {
  phase: 'collecting' | 'validating' | 'complete' | 'error';
  filesCollected: number;
  currentFile?: string;
  progress: number;
  results?: PackageValidationResults;
  fileContents?: Record<string, string>;
  error?: string;
}

/**
 * Print validation results as plain lines (non-TTY / CI output).
 */
function printPlainValidationReport(results: PackageValidationResults): void {
  for (const [filePath, result] of Object.entries(results)) {
    for (const err of result.errors) {
      console.log(`${filePath}: ERROR [${err.error_code}] ${err.message}`);
    }
    for (const warn of result.warnings) {
      console.log(`${filePath}: WARN  [${warn.error_code}] ${warn.message}`);
    }
  }
  const summary = getValidationSummary(results);
  console.log(
    `Validation: ${summary.totalFiles} file(s), ${summary.errorCount} error(s), ${summary.warningCount} warning(s)`
  );
}

/**
 * Run validation with animated progress display (plain lines when stdout
 * is not a TTY, e.g. CI or piped output).
 */
async function runValidationWithProgress(
  packageDir: string,
  env: EnvContext = emptyEnvContext()
): Promise<PackageValidationResults> {
  if (!process.stdout.isTTY) {
    console.log(`Validating package: ${packageDir}`);
    const results = await validatePackageDirectory(packageDir, env);
    printPlainValidationReport(results);
    return results;
  }
  // Initial state
  let state: ValidationState = {
    phase: 'collecting',
    filesCollected: 0,
    progress: 0,
  };

  // Render progress component
  const { unmount, rerender } = render(
    React.createElement(PackageValidator, state)
  );

  const updateState = (newState: Partial<ValidationState>) => {
    state = { ...state, ...newState };
    rerender(React.createElement(PackageValidator, state));
  };

  try {
    // Phase 1: Initialize WASM
    await initSchemaValidator();

    // Phase 2: Collect files
    updateState({ phase: 'collecting' });
    await new Promise(resolve => setTimeout(resolve, 100)); // Brief pause for visual

    const files = collectPackageFiles(packageDir, env);
    const fileCount = Object.keys(files).length;
    updateState({ filesCollected: fileCount });

    await new Promise(resolve => setTimeout(resolve, 200)); // Brief pause for visual

    // Phase 3: Validate
    updateState({ phase: 'validating', progress: 0 });

    // Simulate progress as we validate
    const fileList = Object.keys(files);
    let validated = 0;

    // We can't easily get per-file progress from the WASM module,
    // so we'll do a simple animation while it runs
    const progressInterval = setInterval(() => {
      const progress = Math.min(95, Math.round((validated / fileCount) * 100) + 5);
      updateState({ progress });
    }, 50);

    // Run the actual validation
    const results = await validatePackageDirectory(packageDir, env);

    clearInterval(progressInterval);
    validated = fileCount;

    // Phase 4: Complete
    updateState({
      phase: 'complete',
      progress: 100,
      results,
      fileContents: files,
    });

    // Keep display visible briefly
    await new Promise(resolve => setTimeout(resolve, 500));
    unmount();

    return results;
  } catch (error) {
    updateState({
      phase: 'error',
      error: error instanceof Error ? error.message : String(error),
    });

    await new Promise(resolve => setTimeout(resolve, 1500));
    unmount();

    throw error;
  }
}

/**
 * Creates a .rap package file from a folder
 */
export async function createPackage(
  folderPath: string,
  outputPath?: string,
  options: CreatePackageOptions = {}
): Promise<void> {
  const resolvedFolder = path.resolve(folderPath);

  if (!fs.existsSync(resolvedFolder)) {
    throw new Error(`Folder not found: ${resolvedFolder}`);
  }

  if (!fs.statSync(resolvedFolder).isDirectory()) {
    throw new Error(`Not a directory: ${resolvedFolder}`);
  }

  // Look for manifest.yaml or manifest.yml (backend expects manifest.yaml)
  const manifestPath = ['manifest.yaml', 'manifest.yml']
    .map((name) => path.join(resolvedFolder, name))
    .find((p) => fs.existsSync(p));

  if (!manifestPath) {
    throw new Error('No manifest.yaml or manifest.yml found in folder');
  }

  // Environment for {env:...} substitution — built once and used by both
  // validation and packing, so the two can never see different bytes.
  assertEnvFilesExist(options.envFile);
  const env = loadEnvContext(resolvedFolder, {
    profile: options.env,
    envFiles: options.envFile,
  });

  // Read manifest (substituted, in case the name/version themselves are bound)
  const manifestContent = substituteEnvTokens(
    fs.readFileSync(manifestPath, 'utf-8'),
    env
  ).text;
  const manifest: PackageManifest = yaml.parse(manifestContent);

  if (!manifest.name) {
    throw new Error('Package manifest must have a "name" field');
  }

  if (!manifest.version) {
    throw new Error('Package manifest must have a "version" field');
  }

  // Validate package unless --no-validate is passed
  if (!options.noValidate || options.validateOnly) {
    const validationResults = await runValidationWithProgress(resolvedFolder, env);
    const summary = getValidationSummary(validationResults);

    // If validate-only mode, exit after showing results
    if (options.validateOnly) {
      if (summary.hasErrors) {
        throw new Error(
          `Validation failed with ${summary.errorCount} error(s) and ${summary.warningCount} warning(s).`
        );
      }
      return;
    }

    // Block creation if there are errors
    if (summary.hasErrors) {
      throw new Error(
        `Package validation failed with ${summary.errorCount} error(s). Fix errors before creating package, or use --no-validate to skip.`
      );
    }
  } else {
    console.log('Skipping validation (--no-validate flag used)');
  }

  // Determine output path
  const output = outputPath || path.join(process.cwd(), `${manifest.name}-${manifest.version}.rap`);

  console.log(`\nCreating package: ${manifest.name} v${manifest.version}`);
  console.log(`Source: ${resolvedFolder}`);
  console.log(`Output: ${output}`);

  try {
    // Create a proper ZIP archive
    createZipPackage(resolvedFolder, output, env);
    console.log(`\nPackage created successfully: ${output}`);
  } catch (error) {
    throw new Error(`Failed to create package: ${error instanceof Error ? error.message : String(error)}`);
  }
}

/**
 * Creates an ignore filter from .gitignore and .rapignore files
 */
export function createIgnoreFilter(sourceDir: string): Ignore {
  const ig = ignore();

  // Add default patterns
  ig.add(DEFAULT_IGNORE_PATTERNS);

  // Always ignore the ignore files themselves from the package
  ig.add('.gitignore');
  ig.add('.rapignore');

  // Read .gitignore if it exists
  const gitignorePath = path.join(sourceDir, '.gitignore');
  if (fs.existsSync(gitignorePath)) {
    const gitignoreContent = fs.readFileSync(gitignorePath, 'utf-8');
    ig.add(gitignoreContent);
    console.log('  Using .gitignore patterns');
  }

  // Read .rapignore if it exists (package-specific ignores)
  const rapignorePath = path.join(sourceDir, '.rapignore');
  if (fs.existsSync(rapignorePath)) {
    const rapignoreContent = fs.readFileSync(rapignorePath, 'utf-8');
    ig.add(rapignoreContent);
    console.log('  Using .rapignore patterns');
  }

  return ig;
}

/**
 * Recursively collect all files in a directory
 */
export function collectFiles(dir: string, baseDir: string, files: string[] = []): string[] {
  const entries = fs.readdirSync(dir, { withFileTypes: true });

  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    const relativePath = path.relative(baseDir, fullPath);

    if (entry.isDirectory()) {
      // Recursively collect files from subdirectories
      collectFiles(fullPath, baseDir, files);
    } else {
      files.push(relativePath);
    }
  }

  return files;
}

/**
 * Creates a ZIP package from the source directory, respecting .gitignore and .rapignore.
 *
 * Text files containing `{env:NAME}` tokens are substituted on the way in, so
 * the shipped `.rap` carries resolved values. Everything else is copied
 * byte-for-byte — reading a binary as utf-8 and writing it back would corrupt it.
 *
 * Throws (before writing anything) if any token could not be resolved: a
 * half-written package with a literal `{env:PREVIEW_SERVER}` in a `base_url` is
 * worse than no package at all.
 */
export function createZipPackage(
  sourceDir: string,
  outputPath: string,
  env: EnvContext = emptyEnvContext()
): void {
  const zip = new AdmZip();

  // Create ignore filter from .gitignore and .rapignore
  const ig = createIgnoreFilter(sourceDir);

  // Collect all files
  const allFiles = collectFiles(sourceDir, sourceDir);

  // Filter out ignored files
  const includedFiles = allFiles.filter((file) => !ig.ignores(file));
  const ignoredCount = allFiles.length - includedFiles.length;

  if (ignoredCount > 0) {
    console.log(`  Excluding ${ignoredCount} file(s) based on ignore patterns`);
  }

  const unresolvedEntries: Array<{ path: string; unresolved: UnresolvedToken[] }> = [];
  let substitutedCount = 0;

  // Add each non-ignored file to the archive
  for (const file of includedFiles) {
    const fullPath = path.join(sourceDir, file);
    const dir = path.dirname(file);

    if (isSubstitutableFile(file)) {
      const raw = fs.readFileSync(fullPath, 'utf-8');
      if (hasEnvTokens(raw)) {
        const { text, unresolved } = substituteEnvTokens(raw, env);
        if (unresolved.length > 0) {
          unresolvedEntries.push({ path: file, unresolved });
          continue;
        }
        // addFile takes a full entry name (addLocalFile takes a directory),
        // and zip entries always use forward slashes.
        zip.addFile(file.split(path.sep).join('/'), Buffer.from(text, 'utf-8'));
        substitutedCount++;
        continue;
      }
    }

    zip.addLocalFile(fullPath, dir === '.' ? '' : dir);
  }

  if (unresolvedEntries.length > 0) {
    throw new Error(formatUnresolvedError(unresolvedEntries, env));
  }

  console.log(`  Including ${includedFiles.length} file(s) in package`);
  if (substitutedCount > 0) {
    console.log(`  Resolved {env:...} tokens in ${substitutedCount} file(s)`);
  }

  zip.writeZip(outputPath);
}

/**
 * State for the upload progress display
 */
interface UploadState {
  phase: 'uploading' | 'processing' | 'complete' | 'error';
  progress: number;
  message: string;
  error?: string;
}

/**
 * Uploads a .rap package file to the server with animated progress display
 *
 * @param filePath - Path to the .rap file
 * @param serverUrl - Optional server URL override
 * @param repo - Optional repository name override
 * @param targetPath - Optional target path in repository (e.g., "/my-folder/package-name")
 */
export async function uploadPackage(filePath: string, serverUrl?: string, repo?: string, targetPath?: string, branch = 'main'): Promise<void> {
  const resolvedFile = path.resolve(filePath);

  if (!fs.existsSync(resolvedFile)) {
    throw new Error(`File not found: ${resolvedFile}`);
  }

  if (!resolvedFile.endsWith('.rap')) {
    throw new Error('File must have .rap extension');
  }

  const server = serverUrl || getServer();
  if (!server) {
    throw new Error('No server configured. Use --server option or run "raisindb shell" and use /connect first.');
  }

  const token = getToken();
  if (!token) {
    throw new Error('Not authenticated. Run "raisindb shell" and use /login first.');
  }

  // Get repo from env/config - require explicit specification if not configured
  const targetRepo = repo || getDefaultRepo();

  if (!targetRepo) {
    throw new Error('No repository specified. Use --repo <name>, set RAISINDB_REPO, or set a default in "raisindb shell" with "use <database>".');
  }

  const fileName = path.basename(resolvedFile);

  // Initial state
  let state: UploadState = {
    phase: 'uploading',
    progress: 0,
    message: 'Starting upload...',
  };

  // Create a wrapper component that tracks state
  const ProgressWrapper = () => {
    const [currentState, setCurrentState] = React.useState<UploadState>(state);

    // Expose setter for state updates
    (ProgressWrapper as any).updateState = (newState: Partial<UploadState>) => {
      state = { ...state, ...newState };
      setCurrentState(state);
    };

    return React.createElement(UploadProgress, {
      phase: currentState.phase,
      progress: currentState.progress,
      fileName,
      message: currentState.message,
      error: currentState.error,
    });
  };

  // Render the progress component
  const { unmount, rerender } = render(React.createElement(ProgressWrapper));

  // Helper to update state and rerender
  const updateProgress = (newState: Partial<UploadState>) => {
    state = { ...state, ...newState };
    rerender(React.createElement(UploadProgress, {
      phase: state.phase,
      progress: state.progress,
      fileName,
      message: state.message,
      error: state.error,
    }));
  };

  try {
    // Read the package file as binary Buffer
    const fileContent = fs.readFileSync(resolvedFile);
    const fileSize = fileContent.length;

    // Simulate upload progress (actual progress would need stream tracking)
    updateProgress({ progress: 10, message: 'Reading file...' });

    // Small delay to show the animation
    await new Promise(resolve => setTimeout(resolve, 200));
    updateProgress({ progress: 30, message: 'Uploading to server...' });

    // Use the api.ts uploadPackage function
    const result = await apiUploadPackage(targetRepo, fileContent, fileName, targetPath, branch);

    updateProgress({ progress: 60, message: 'Upload received by server' });

    // If we got a job_id, track the background processing via SSE
    if (result.job_id) {
      updateProgress({
        phase: 'processing',
        progress: 0,
        message: 'Processing package...',
      });

      // Wait for job completion via SSE
      await new Promise<void>((resolve, reject) => {
        const cleanup = subscribeToJobEvents(
          (event: JobEvent) => {
            // Only handle events for our job
            if (event.job_id !== result.job_id) return;

            // Update progress
            if (event.progress !== null) {
              updateProgress({
                progress: Math.round(event.progress * 100),
                message: `Processing... ${Math.round(event.progress * 100)}%`,
              });
            }

            // Check for completion
            if (event.status === 'Completed') {
              cleanup();
              updateProgress({
                phase: 'complete',
                progress: 100,
                message: `Package '${result.name}' uploaded and processed successfully!`,
              });
              resolve();
            } else if (event.status.startsWith('Failed')) {
              cleanup();
              const errorMsg = event.error || event.status.replace('Failed: ', '');
              updateProgress({
                phase: 'error',
                error: errorMsg,
              });
              reject(new Error(errorMsg));
            }
          },
          (error) => {
            // SSE connection error - fall back to assuming success
            cleanup();
            updateProgress({
              phase: 'complete',
              progress: 100,
              message: `Package '${result.name}' uploaded (processing status unknown)`,
            });
            resolve();
          }
        );

        // Timeout after 5 minutes
        setTimeout(() => {
          cleanup();
          updateProgress({
            phase: 'complete',
            progress: 100,
            message: `Package '${result.name}' uploaded (still processing in background)`,
          });
          resolve();
        }, 5 * 60 * 1000);
      });
    } else {
      // Small upload - already processed
      updateProgress({
        phase: 'complete',
        progress: 100,
        message: `Package '${result.name}' uploaded successfully!`,
      });
    }

    // Keep the final state visible for a moment
    await new Promise(resolve => setTimeout(resolve, 1000));
    unmount();
  } catch (error) {
    updateProgress({
      phase: 'error',
      error: error instanceof Error ? error.message : String(error),
    });

    // Keep error visible
    await new Promise(resolve => setTimeout(resolve, 2000));
    unmount();

    throw new Error(`Failed to upload package: ${error instanceof Error ? error.message : String(error)}`);
  }
}

/**
 * Lists installed packages (requires server connection)
 */
export async function listPackages(serverUrl?: string, repo?: string): Promise<void> {
  const server = serverUrl || getServer();
  if (!server) {
    throw new Error('No server configured. Use --server option or run /connect first.');
  }

  const token = getToken();
  if (!token) {
    throw new Error('Not authenticated. Run /login first.');
  }

  // Get repo from env/config or use 'default'
  const targetRepo = repo || getDefaultRepo() || 'default';

  try {
    const packages = await apiListPackages(targetRepo);

    if (packages.length === 0) {
      console.log('No packages found.');
      return;
    }

    console.log(`\nPackages in repository '${targetRepo}':\n`);
    console.log('  Name                          Version     Installed   Status');
    console.log('  ──────────────────────────────────────────────────────────────');

    for (const pkg of packages) {
      const name = (pkg.name || 'unknown').padEnd(30);
      const version = (pkg.version || '-').padEnd(12);
      const installed = (pkg.installed ? '✓' : '-').padEnd(12);
      // Builtin packages installed at repo creation have no status property
      const status = pkg.status || (pkg.installed ? 'installed' : '-');
      console.log(`  ${name}${version}${installed}${status}`);
      if (pkg.status === 'failed' && pkg.error) {
        console.log(`    error: ${pkg.error}`);
      }
    }

    console.log(`\n  Total: ${packages.length} package(s)`);
  } catch (error) {
    throw new Error(`Failed to list packages: ${error instanceof Error ? error.message : String(error)}`);
  }
}

/** How long to wait for an install job to reach a terminal state */
const INSTALL_POLL_TIMEOUT_MS = 180_000;
const INSTALL_POLL_INTERVAL_MS = 1_000;

/**
 * Poll the package list until the named package reaches a terminal install
 * state. Success criterion is status === 'installed' (not just job
 * completion); status === 'failed' throws with the server's error detail.
 */
async function waitForInstalled(
  repo: string,
  packageName: string,
  jobId?: string | null,
  timeoutMs: number = INSTALL_POLL_TIMEOUT_MS
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let lastStatus = '';

  while (Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, INSTALL_POLL_INTERVAL_MS));

    let pkg;
    try {
      const packages = await apiListPackages(repo);
      pkg = packages.find((p) => p.name === packageName);
    } catch {
      continue; // transient network/API error — keep polling
    }
    if (!pkg) continue;

    const status = pkg.status || (pkg.installed ? 'installed' : '');
    if (status && status !== lastStatus) {
      console.log(`  status: ${status}`);
      lastStatus = status;
    }

    if (status === 'installed') {
      return;
    }
    if (status === 'failed') {
      // Prefer the error on the package node; fall back to the job record
      // (older repos may have a raisin:Package schema without `error`).
      let detail = pkg.error;
      if (!detail && jobId) {
        const job = await apiGetJobInfo(jobId);
        detail = job?.error || undefined;
      }
      throw new Error(
        `Package '${packageName}' install failed${detail ? `: ${detail}` : ''}`
      );
    }
  }

  throw new Error(
    `Timed out waiting for package '${packageName}' to install (last status: ${lastStatus || 'unknown'})`
  );
}

/**
 * Installs a package by name (requires server connection).
 *
 * Waits for the background install job and reports the final lifecycle
 * state ('installed' = success, 'failed' = error with detail).
 */
export async function installPackage(
  packageName: string,
  serverUrl?: string,
  repo?: string,
  branch = 'main',
  mode?: PackageInstallMode
): Promise<void> {
  const server = serverUrl || getServer();
  if (!server) {
    throw new Error('No server configured. Use --server option or run /connect first.');
  }

  const token = getToken();
  if (!token) {
    throw new Error('Not authenticated. Run "raisindb login" first.');
  }

  // Get repo from env/config or use 'default'
  const targetRepo = repo || getDefaultRepo() || 'default';

  console.log(`Installing package '${packageName}' in repository '${targetRepo}' (branch: ${branch})...`);

  try {
    const response = await apiInstallPackage(targetRepo, packageName, branch, mode);

    if (response.installed && !response.job_id) {
      // The server declined to do anything: the package is already installed
      // and the mode says leave existing content alone. Say so plainly — this
      // used to print "is already installed" and exit 0, which read as success
      // while not one content node had been looked at.
      console.log(
        `\nPackage '${packageName}' is already installed, and mode '${mode || 'skip'}' ` +
          'leaves existing content untouched — nothing was applied.'
      );
      console.log("Re-apply the package's content with: --mode sync");
      return;
    }

    await waitForInstalled(targetRepo, packageName, response.job_id);
    console.log(`\nPackage '${packageName}' installed successfully!`);
  } catch (error) {
    throw new Error(`Failed to install package: ${error instanceof Error ? error.message : String(error)}`);
  }
}
