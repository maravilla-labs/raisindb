/**
 * Loading the environment used by `{env:NAME}` substitution.
 *
 * Values come from `.env` files in the package directory plus the process
 * environment. The process environment always wins, so CI needs no files.
 */

import fs from 'fs';
import path from 'path';
import dotenv from 'dotenv';
import { EnvContext } from './substitute.js';

export interface EnvLoadOptions {
  /** Profile selector, e.g. `--env production` → `.env.production`. */
  profile?: string;
  /** Explicit files from `--env-file`, applied in order, after the conventional ones. */
  envFiles?: string[];
}

/**
 * File extensions whose contents are treated as text and substituted.
 *
 * Everything else (images, fonts, archives) is copied byte-for-byte — reading a
 * binary as utf-8 and writing it back would corrupt it.
 */
export const SUBSTITUTABLE_EXTENSIONS = [
  '.yaml',
  '.yml',
  '.json',
  '.js',
  '.py',
  '.star',
  '.md',
];

export function isSubstitutableFile(filePath: string): boolean {
  return SUBSTITUTABLE_EXTENSIONS.includes(path.extname(filePath).toLowerCase());
}

/**
 * Build the substitution environment for a package directory.
 *
 * Precedence, lowest to highest:
 *   1. <packageDir>/.env
 *   2. <packageDir>/.env.<profile>          (only with --env <profile>)
 *   3. <packageDir>/.env.local
 *   4. <packageDir>/.env.<profile>.local    (only with --env <profile>)
 *   5. each --env-file, in the order given
 *   6. process.env
 */
export function loadEnvContext(packageDir: string, options: EnvLoadOptions = {}): EnvContext {
  const { profile, envFiles = [] } = options;

  const candidates = [
    path.join(packageDir, '.env'),
    ...(profile ? [path.join(packageDir, `.env.${profile}`)] : []),
    path.join(packageDir, '.env.local'),
    ...(profile ? [path.join(packageDir, `.env.${profile}.local`)] : []),
    ...envFiles.map((f) => path.resolve(f)),
  ];

  const values: Record<string, string> = {};
  const sources: string[] = [];

  for (const candidate of candidates) {
    if (!fs.existsSync(candidate) || !fs.statSync(candidate).isFile()) {
      continue;
    }
    try {
      const parsed = dotenv.parse(fs.readFileSync(candidate));
      Object.assign(values, parsed);
      sources.push(candidate);
    } catch (error) {
      throw new Error(
        `Failed to read env file ${candidate}: ${error instanceof Error ? error.message : String(error)}`
      );
    }
  }

  // The real environment wins over every file.
  for (const [key, value] of Object.entries(process.env)) {
    if (value !== undefined) {
      values[key] = value;
    }
  }
  sources.push('process environment');

  return { values, sources };
}

/**
 * An explicitly requested --env-file that does not exist is a mistake worth
 * reporting; a missing conventional .env is not.
 */
export function assertEnvFilesExist(envFiles: string[] = []): void {
  for (const file of envFiles) {
    const resolved = path.resolve(file);
    if (!fs.existsSync(resolved)) {
      throw new Error(`Env file not found: ${resolved}`);
    }
  }
}
