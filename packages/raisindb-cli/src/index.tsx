#!/usr/bin/env node
import React from 'react';
import { render } from 'ink';
import { Command } from 'commander';
import App from './app.js';
import { createPackage, uploadPackage, listPackages, installPackage } from './commands/package.js';
import { syncPackage } from './commands/sync.js';
import { clonePackage } from './commands/clone.js';
import { createFromServer } from './commands/create-from-server.js';
import { initPackage } from './commands/init.js';
import { serverInstall, serverStart, serverVersion, serverUpdate } from './commands/server.js';

const program = new Command();

program
  .name('raisindb')
  .description('RaisinDB CLI - Interactive terminal interface for RaisinDB')
  .version('0.1.0');

// Package commands (offline)
const packageCmd = program
  .command('package')
  .description('Package management commands');

packageCmd
  .command('create <folder>')
  .description('Create a .rap package from a folder')
  .option('-o, --output <file>', 'Output file path')
  .option('--no-validate', 'Skip schema validation')
  .option('--check', 'Only validate (check), do not create package')
  .action(async (folder, options) => {
    try {
      // Commander.js: --no-validate sets options.validate = false
      //               --check sets options.check = true (validate only)
      //               no flag: validate then create
      await createPackage(folder, options.output, {
        noValidate: options.validate === false,  // --no-validate was passed
        validateOnly: options.check === true     // --check was passed
      });
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

packageCmd
  .command('upload <file>')
  .description('Upload a .rap package to the server')
  .option('-s, --server <url>', 'Server URL')
  .option('-r, --repo <name>', 'Repository name (default: from config or "default")')
  .option('-p, --path <path>', 'Target path in repository (e.g., /my-folder/package-name)')
  .action(async (file, options) => {
    try {
      await uploadPackage(file, options.server, options.repo, options.path);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

packageCmd
  .command('list')
  .description('List packages in a repository')
  .option('-s, --server <url>', 'Server URL')
  .option('-r, --repo <name>', 'Repository name (default: from config or "default")')
  .action(async (options) => {
    try {
      await listPackages(options.server, options.repo);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

packageCmd
  .command('install <name>')
  .description('Install a package by name')
  .option('-s, --server <url>', 'Server URL')
  .option('-r, --repo <name>', 'Repository name (default: from config or "default")')
  .action(async (name, options) => {
    try {
      await installPackage(name, options.server, options.repo);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

packageCmd
  .command('sync [directory]')
  .description('Synchronize package with server')
  .option('-w, --watch', 'Enable continuous file watching mode')
  .option('-p, --push', 'One-way sync: local to server only')
  .option('-l, --pull', 'One-way sync: server to local only')
  .option('-y, --yes', 'Skip interactive confirmations')
  .option('-f, --force', 'Overwrite conflicts without prompting')
  .option('-n, --dry-run', 'Show what would be synced without making changes')
  .option('-r, --repo <name>', 'Target repository')
  .option('-s, --server <url>', 'Server URL')
  .option('--init', 'Initialize sync configuration')
  .action(async (directory, options) => {
    try {
      await syncPackage(directory || process.cwd(), options);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

packageCmd
  .command('clone [name]')
  .description('Clone a package from server to local directory')
  .option('-o, --output <dir>', 'Output directory (default: ./<package-name>)')
  .option('-s, --server <url>', 'Server URL')
  .option('-r, --repo <name>', 'Repository name (default: from config or "default")')
  .option('-b, --branch <name>', 'Branch name (default: "main")')
  .action(async (name, options) => {
    try {
      await clonePackage(name, options);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

packageCmd
  .command('create-from-server')
  .description('Create a new package by selecting content from server')
  .option('-s, --server <url>', 'Server URL')
  .option('-r, --repo <name>', 'Repository name (default: from config or "default")')
  .action(async (options) => {
    try {
      await createFromServer(options);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

packageCmd
  .command('init <folder>')
  .description('Initialize a new package with agent coding instructions')
  .option('--pack <name>', 'Template pack (default: content-modeling)', 'content-modeling')
  .option('-n, --name <name>', 'Package name (default: folder name)')
  .option('-w, --workspace <name>', 'Workspace name (default: package name)')
  .option('-d, --description <text>', 'Package description')
  .action(async (folder, options) => {
    try {
      await initPackage(folder, options);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

// Server commands
const serverCmd = program
  .command('server')
  .description('Manage the RaisinDB server binary');

serverCmd
  .command('install')
  .description('Download and install the RaisinDB server binary')
  .option('-v, --version <tag>', 'Install a specific version (e.g., v0.1.0)')
  .option('-f, --force', 'Force reinstall even if already installed')
  .action(async (options) => {
    try {
      await serverInstall({ version: options.version, force: options.force });
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

serverCmd
  .command('start')
  .description('Start the RaisinDB server (installs if needed)')
  .option('--port <port>', 'HTTP port')
  .option('--pgwire-enabled <bool>', 'Enable PostgreSQL wire protocol')
  .option('--config <path>', 'Path to config file')
  .allowUnknownOption(true)
  .action(async (options, command) => {
    try {
      // Pass all arguments after 'server start' to the binary
      const args = command.args || [];
      const passthrough: string[] = [];
      if (options.port) passthrough.push('--port', options.port);
      if (options.pgwireEnabled) passthrough.push('--pgwire-enabled', options.pgwireEnabled);
      if (options.config) passthrough.push('--config', options.config);
      passthrough.push(...args);
      await serverStart(passthrough);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

serverCmd
  .command('update')
  .description('Update the RaisinDB server to the latest version')
  .action(async () => {
    try {
      await serverUpdate();
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

serverCmd
  .command('version')
  .description('Show installed server version')
  .action(async () => {
    try {
      await serverVersion();
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

// Default: Interactive shell mode
program
  .command('shell', { isDefault: true })
  .description('Start interactive shell (default)')
  .option('-s, --server <url>', 'Server URL to connect to')
  .option('-d, --database <name>', 'Database to use')
  .action((options) => {
    const { waitUntilExit } = render(
      <App serverUrl={options.server} database={options.database} />,
      { exitOnCtrlC: false }  // We handle Ctrl+C ourselves
    );

    waitUntilExit().then(() => {
      process.exit(0);
    });
  });

program.parse();
