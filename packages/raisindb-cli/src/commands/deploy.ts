import fs from 'fs';
import path from 'path';
import yaml from 'yaml';
import { createPackage } from './package.js';
import { uploadPackage, installPackage } from './package.js';
import { getDefaultRepo } from '../config.js';
import { getWorkspaceRaw, putWorkspaceRaw, type PackageInstallMode } from '../api.js';
import { EnvContext, emptyEnvContext, substituteEnvTokens } from '../env/substitute.js';
import { assertEnvFilesExist, loadEnvContext } from '../env/load.js';

interface DeployOptions {
  server?: string;
  repo?: string;
  /** Target branch to upload the package to (defaults to "main"). */
  branch?: string;
  /** Also install the package after upload (deploy alone only uploads). */
  install?: boolean;
  /**
   * How the install treats content that already exists on the server.
   *
   * Defaults to `sync` — re-deploying a package is meant to APPLY the package,
   * and the server's own default of `skip` made `deploy --install` over an
   * existing package do nothing at all while reporting success.
   */
  mode?: PackageInstallMode;
  /** Profile for {env:...} substitution: --env production → .env.production */
  env?: string;
  /** Extra env files from --env-file, applied after the conventional ones. */
  envFile?: string[];
}

/**
 * Deploy a package: validate → build .rap → upload to server.
 * With `--install` it also installs the uploaded package.
 * Reads manifest.yaml for the package name and version automatically.
 */
export async function deployPackage(folder: string, options: DeployOptions): Promise<void> {
  const resolvedFolder = path.resolve(folder);

  if (!fs.existsSync(resolvedFolder) || !fs.statSync(resolvedFolder).isDirectory()) {
    throw new Error(`Folder not found: ${resolvedFolder}`);
  }

  // Read manifest to get name + version for the .rap filename
  const manifestPath = ['manifest.yaml', 'manifest.yml']
    .map((name) => path.join(resolvedFolder, name))
    .find((p) => fs.existsSync(p));

  if (!manifestPath) {
    throw new Error('No manifest.yaml or manifest.yml found in folder');
  }

  assertEnvFilesExist(options.envFile);
  const env = loadEnvContext(resolvedFolder, {
    profile: options.env,
    envFiles: options.envFile,
  });

  const manifest = yaml.parse(
    substituteEnvTokens(fs.readFileSync(manifestPath, 'utf-8'), env).text
  );
  if (!manifest.name || !manifest.version) {
    throw new Error('Package manifest must have "name" and "version" fields');
  }

  const rapFile = path.join(process.cwd(), `${manifest.name}-${manifest.version}.rap`);

  // Step 1+2: Validate and create .rap (createPackage does both)
  console.log(`\nDeploying ${manifest.name} v${manifest.version}...\n`);
  await createPackage(resolvedFolder, rapFile, {
    env: options.env,
    envFile: options.envFile,
  });

  // Step 3: Upload
  console.log(`\nUploading ${path.basename(rapFile)}...`);
  await uploadPackage(rapFile, options.server, options.repo, undefined, options.branch || 'main');

  // Clean up .rap file
  if (fs.existsSync(rapFile)) {
    fs.unlinkSync(rapFile);
  }

  // Step 4 (optional): Install
  if (options.install) {
    await installPackage(
      manifest.name,
      options.server,
      options.repo,
      options.branch || 'main',
      options.mode || 'sync'
    );

    // Step 5: Reconcile EXISTING workspaces' allowed types. Package install
    // seeds a NEW workspace with the full definition, but for an already-
    // registered workspace it does not update allowed_root_node_types (and only
    // partially allowed_node_types), so the package YAML drifts from the server.
    // Re-apply both from each workspaces/*.yaml over the (operator) HTTP API.
    const targetRepo = options.repo || getDefaultRepo() || 'default';
    await reconcileWorkspaceAllowedTypes(resolvedFolder, targetRepo, env);
  }

  console.log(`\nDeployed ${manifest.name} v${manifest.version} successfully${options.install ? ' (installed)' : ''}.`);
}

/**
 * The package's types, then every type the server already allows that the
 * package does not list. A deploy NEVER NARROWS a workspace: a type the
 * installation added since — an app Studio Builder made, an operator's
 * addition — stays allowed, because dropping it strands every existing node of
 * that type in a workspace that now refuses it. The server's `sync` install
 * merges the same way; this step used to replace the lists and undo that.
 * `null` when nothing would change.
 */
function widened(existing: unknown, fromPackage: string[]): string[] | null {
  const have = Array.isArray(existing) ? (existing as unknown[]).filter((v): v is string => typeof v === 'string') : [];
  if (fromPackage.every((t) => have.includes(t))) return null;
  return [...fromPackage, ...have.filter((t) => !fromPackage.includes(t))];
}

/**
 * Re-apply each package workspace's `allowed_node_types` /
 * `allowed_root_node_types` to the server for workspaces that already exist,
 * so a `deploy --install` over a pre-existing workspace picks up allowed-type
 * changes (the server install path only fully seeds NEW workspaces).
 *
 * Reads `<folder>/workspaces/*.yaml` (the workspace-definition files), and for
 * each that already exists on the server and is MISSING a type the package
 * declares, GETs the current workspace, widens the two arrays (see `widened` —
 * never narrowed) and PUTs it back, preserving all other fields. New
 * workspaces are left to the install path.
 */
export async function reconcileWorkspaceAllowedTypes(
  folder: string,
  repo: string,
  env: EnvContext = emptyEnvContext()
): Promise<void> {
  const wsDir = path.join(folder, 'workspaces');
  if (!fs.existsSync(wsDir) || !fs.statSync(wsDir).isDirectory()) return;

  const files = fs
    .readdirSync(wsDir, { withFileTypes: true })
    .filter((e) => e.isFile() && /\.ya?ml$/i.test(e.name))
    .map((e) => path.join(wsDir, e.name));
  if (files.length === 0) return;

  let reconciled = 0;
  for (const file of files) {
    let def: Record<string, unknown>;
    try {
      def = (yaml.parse(
        substituteEnvTokens(fs.readFileSync(file, 'utf-8'), env).text
      ) ?? {}) as Record<string, unknown>;
    } catch {
      continue; // malformed YAML is a validation concern, not this reconciler's
    }

    const name =
      (typeof def.name === 'string' && def.name) || path.basename(file).replace(/\.ya?ml$/i, '');
    const nodeTypes = def.allowed_node_types;
    const rootTypes = def.allowed_root_node_types;
    // Nothing to reconcile if the package doesn't declare allowed types.
    if (!Array.isArray(nodeTypes) && !Array.isArray(rootTypes)) continue;

    let existing: Record<string, unknown> | null;
    try {
      existing = await getWorkspaceRaw(repo, name);
    } catch (e) {
      console.warn(`  ⚠ could not read workspace '${name}': ${e instanceof Error ? e.message : e}`);
      continue;
    }
    if (!existing) continue; // new workspace — install already seeded it fully

    const updated = { ...existing };
    let changed = false;
    const nextTypes = Array.isArray(nodeTypes) ? widened(existing.allowed_node_types, nodeTypes as string[]) : null;
    if (nextTypes) {
      updated.allowed_node_types = nextTypes;
      changed = true;
    }
    const nextRoots = Array.isArray(rootTypes) ? widened(existing.allowed_root_node_types, rootTypes as string[]) : null;
    if (nextRoots) {
      updated.allowed_root_node_types = nextRoots;
      changed = true;
    }
    if (!changed) continue;

    try {
      await putWorkspaceRaw(repo, name, updated);
      reconciled++;
      console.log(`  ↳ reconciled workspace '${name}' allowed types`);
    } catch (e) {
      console.warn(
        `  ⚠ failed to reconcile workspace '${name}': ${e instanceof Error ? e.message : e}`
      );
    }
  }

  if (reconciled > 0) {
    console.log(`Reconciled allowed types for ${reconciled} existing workspace(s).`);
  }
}
