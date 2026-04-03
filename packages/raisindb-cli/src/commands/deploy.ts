import fs from 'fs';
import path from 'path';
import yaml from 'yaml';
import { createPackage } from './package.js';
import { uploadPackage } from './package.js';

interface DeployOptions {
  server?: string;
  repo?: string;
}

/**
 * Deploy a package: validate → build .rap → upload to server.
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

  const manifest = yaml.parse(fs.readFileSync(manifestPath, 'utf-8'));
  if (!manifest.name || !manifest.version) {
    throw new Error('Package manifest must have "name" and "version" fields');
  }

  const rapFile = path.join(process.cwd(), `${manifest.name}-${manifest.version}.rap`);

  // Step 1+2: Validate and create .rap (createPackage does both)
  console.log(`\nDeploying ${manifest.name} v${manifest.version}...\n`);
  await createPackage(resolvedFolder, rapFile);

  // Step 3: Upload
  console.log(`\nUploading ${path.basename(rapFile)}...`);
  await uploadPackage(rapFile, options.server, options.repo);

  // Clean up .rap file
  if (fs.existsSync(rapFile)) {
    fs.unlinkSync(rapFile);
  }

  console.log(`\nDeployed ${manifest.name} v${manifest.version} successfully.`);
}
