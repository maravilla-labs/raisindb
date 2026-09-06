#!/usr/bin/env node
import React from 'react';
import { render } from 'ink';
import { Command } from 'commander';
import { createRequire } from 'module';
import App from './app.js';
import { createPackage, uploadPackage, listPackages, installPackage } from './commands/package.js';
import { syncPackage } from './commands/sync.js';
import { clonePackage } from './commands/clone.js';
import { createFromServer } from './commands/create-from-server.js';
import { initPackage } from './commands/init.js';
import { createAdapter } from './commands/create.js';
import { createFunction } from './commands/create-function.js';
import { functionBuild, functionDoctor } from './commands/function.js';
import { functionRun, functionTest } from './commands/function-run.js';
import { deployPackage } from './commands/deploy.js';
import { serverInstall, serverStart, serverVersion, serverUpdate, serverStop, serverStatus, serverLogs } from './commands/server.js';
import { flowDoctor, flowExplain } from './commands/flow.js';
import { repoCreate, repoList, repoDelete } from './commands/repo.js';
import { aiProviderSet, aiProviderList, aiProviderTest } from './commands/ai.js';
import { userRegister } from './commands/user.js';
import { corsAdd, corsList, corsRemove } from './commands/cors.js';
import { secretSet, secretList, secretShow, secretRotate, secretRemove } from './commands/secret.js';
import { login, logout, isAuthenticated, loginWithPassword, loginWithToken } from './auth.js';
import { getServer } from './config.js';
import { HelpDisplay } from './components/HelpDisplay.js';

const require = createRequire(import.meta.url);
const pkg = require('../package.json');

const program = new Command();

program
  .name('raisindb')
  .description('RaisinDB CLI - Interactive terminal interface for RaisinDB')
  .version(pkg.version);

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
  .option('-e, --env <profile>', 'Env profile for {env:...} tokens (loads .env.<profile>)')
  .option('--env-file <path...>', 'Additional env file(s) for {env:...} tokens')
  .action(async (folder, options) => {
    try {
      // Commander.js: --no-validate sets options.validate = false
      //               --check sets options.check = true (validate only)
      //               no flag: validate then create
      await createPackage(folder, options.output, {
        noValidate: options.validate === false,  // --no-validate was passed
        validateOnly: options.check === true,    // --check was passed
        env: options.env,
        envFile: options.envFile,
      });
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

packageCmd
  .command('validate <folder>')
  .description('Validate a package folder (schema + flow doctor) without building a .rap')
  .option('-e, --env <profile>', 'Env profile for {env:...} tokens (loads .env.<profile>)')
  .option('--env-file <path...>', 'Additional env file(s) for {env:...} tokens')
  .action(async (folder, options) => {
    try {
      await createPackage(folder, undefined, {
        validateOnly: true,
        env: options.env,
        envFile: options.envFile,
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
  .option('-b, --branch <name>', 'Target branch (default: "main")')
  .action(async (name, options) => {
    try {
      await installPackage(name, options.server, options.repo, options.branch || 'main');
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
  .option('-b, --branch <name>', 'Target branch (default: "main")')
  .option('--init', 'Initialize sync configuration')
  .option('-e, --env <profile>', 'Env profile for {env:...} tokens (loads .env.<profile>)')
  .option('--env-file <path...>', 'Additional env file(s) for {env:...} tokens')
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
  .option('--skip-install', 'Skip npm install and skills installation')
  .action(async (folder, options) => {
    try {
      await initPackage(folder, options);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

packageCmd
  .command('deploy <folder>')
  .description('Validate, build, and upload package in one step')
  .option('-s, --server <url>', 'Server URL')
  .option('-r, --repo <name>', 'Repository name')
  .option('-b, --branch <name>', 'Target branch (default: "main")')
  .option('-i, --install', 'Install the package after upload')
  .option(
    '--mode <mode>',
    'With --install, how to treat existing content: skip | sync | overwrite',
    'sync'
  )
  .option('-e, --env <profile>', 'Env profile for {env:...} tokens (loads .env.<profile>)')
  .option('--env-file <path...>', 'Additional env file(s) for {env:...} tokens')
  .action(async (folder, options) => {
    try {
      await deployPackage(folder, options);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

// Scaffolding commands (offline). Grouped under the `create` verb so new
// generators can be added as subcommands (e.g. `raisindb create adapter`).
const createCmd = program
  .command('create')
  .description('Scaffold new RaisinDB artifacts');

createCmd
  .command('adapter <name>')
  .description('Scaffold a connector-adapter package skeleton')
  .option('-d, --dir <path>', 'Target directory (default: ./<name>-adapter)')
  .option('-p, --provider <slug>', 'Provider type slug (default: <name>)')
  .option('--description <text>', 'Package description')
  .action(async (name, options) => {
    try {
      await createAdapter(name, options);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

createCmd
  .command('function <name>')
  .description('Scaffold a WebAssembly function (rust | go | ts) inside a package')
  .option('-l, --lang <lang>', 'Guest language: rust | go | ts')
  .option('--ns <namespace>', 'Namespace under content/functions/lib (default: package name)')
  .option('-d, --dir <path>', 'Package directory (default: nearest manifest.yaml above cwd)')
  .option('--handler <name>', 'Handler name (default: "default", or <name> with --into)')
  .option(
    '--into <project>',
    'Add this function as a SECOND handler of an existing wasm project, sharing its artifact'
  )
  .option('--description <text>', 'One-line description for the Function node')
  .action(async (name, options) => {
    try {
      await createFunction(name, options);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

// WebAssembly function commands. `build`/`doctor` are offline (toolchain +
// local files only); `run` and `test --server` talk to a server.
const functionCmd = program
  .command('function')
  .description('WebAssembly function tools: build, doctor, run, test');

functionCmd
  .command('build [path]')
  .description('Build a wasm function project and copy the artifact into its Function node')
  .option('--all', 'Build every wasm project in the package')
  .option('-w, --watch', 'Rebuild on change until interrupted')
  .option('--release', 'Build with the release profile (the default)')
  .option('--debug', 'Build with the debug profile — faster, much larger artifacts')
  .action(async (target, options) => {
    try {
      process.exit(await functionBuild(target, options));
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(2);
    }
  });

functionCmd
  .command('doctor [path]')
  .description('Check toolchains, entry_file handler names and artifact size')
  .option('--json', 'Machine-readable JSON output')
  .option('--strict', 'Treat warnings as failures (non-zero exit)')
  .action((target, options) => {
    try {
      process.exit(functionDoctor(target, { json: options.json, strict: options.strict }));
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(2);
    }
  });

functionCmd
  .command('run [path]')
  .description('Run a wasm function against a server (uploads the local artifact when it differs)')
  .option('-i, --input <json>', 'JSON input for the handler')
  .option('--input-file <path>', 'Read the JSON input from a file')
  .option('--handler <name>', 'Call this handler instead of the node\'s entry_file one')
  .option('-t, --timeout <ms>', 'Timeout in milliseconds')
  .option('-s, --server <url>', 'Server URL')
  .option('-r, --repo <name>', 'Repository name')
  .option('-b, --branch <name>', 'Branch to upload the artifact to (default: "main")')
  .option('--json', 'Print one JSON object instead of the live view')
  .action(async (target, options) => {
    try {
      process.exit(await functionRun(target, options));
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(2);
    }
  });

functionCmd
  .command('test [path]')
  .description("Run a wasm project's native tests, or its tests/server.json with --server")
  .option('--server [url]', 'Replay tests/server.json against a running server')
  .option('-r, --repo <name>', 'Repository name (with --server)')
  .option('-b, --branch <name>', 'Branch to upload the artifact to (with --server)')
  .option('-t, --timeout <ms>', 'Per-case timeout in milliseconds')
  .action(async (target, options) => {
    try {
      process.exit(await functionTest(target, options));
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(2);
    }
  });

// Top-level aliases for the core dev-loop commands:
//   raisindb deploy ./package --repo myapp --install
//   raisindb sync ./package --repo myapp --watch
program
  .command('deploy <folder>')
  .description('Validate, build, and upload a package (alias for "package deploy")')
  .option('-s, --server <url>', 'Server URL')
  .option('-r, --repo <name>', 'Repository name')
  .option('-b, --branch <name>', 'Target branch (default: "main")')
  .option('-i, --install', 'Install the package after upload')
  .option(
    '--mode <mode>',
    'With --install, how to treat existing content: skip | sync | overwrite',
    'sync'
  )
  .option('-e, --env <profile>', 'Env profile for {env:...} tokens (loads .env.<profile>)')
  .option('--env-file <path...>', 'Additional env file(s) for {env:...} tokens')
  .action(async (folder, options) => {
    try {
      await deployPackage(folder, options);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

program
  .command('sync [directory]')
  .description('Synchronize a package directory with the server (alias for "package sync")')
  .option('-w, --watch', 'Enable continuous file watching mode')
  .option('-p, --push', 'One-way sync: local to server only')
  .option('-l, --pull', 'One-way sync: server to local only')
  .option('-y, --yes', 'Skip interactive confirmations')
  .option('-f, --force', 'Overwrite conflicts without prompting')
  .option('-n, --dry-run', 'Show what would be synced without making changes')
  .option('-r, --repo <name>', 'Target repository')
  .option('-s, --server <url>', 'Server URL')
  .option('-b, --branch <name>', 'Target branch (default: "main")')
  .option('--init', 'Initialize sync configuration')
  .option('-e, --env <profile>', 'Env profile for {env:...} tokens (loads .env.<profile>)')
  .option('--env-file <path...>', 'Additional env file(s) for {env:...} tokens')
  .action(async (directory, options) => {
    try {
      await syncPackage(directory || process.cwd(), options);
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

// Flow commands (offline static analysis)
const flowCmd = program
  .command('flow')
  .description('Workflow authoring tools (static analysis, no server needed)');

flowCmd
  .command('doctor <path>')
  .description('Statically analyze flow definitions (flow YAML file or package folder)')
  .option('--json', 'Machine-readable JSON output')
  .option('--strict', 'Treat warnings as failures (non-zero exit)')
  .action((target, options) => {
    try {
      const exitCode = flowDoctor(target, { json: options.json, strict: options.strict });
      process.exit(exitCode);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(2);
    }
  });

flowCmd
  .command('explain <path>')
  .description('Print the lowered execution plan the engine will actually run')
  .action((target) => {
    try {
      process.exit(flowExplain(target));
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(2);
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
  .description('Start the RaisinDB server (dev mode by default)')
  .option('--port <port>', 'HTTP port (default: 8080)')
  .option('--pgwire-port <port>', 'PostgreSQL port (default: 5432)')
  .option('--config <path>', 'Path to config file')
  .option('--production', 'Production mode (requires secrets)')
  .option('--verbose', 'Show server logs in terminal')
  .option('-d, --detach', 'Run in background')
  .allowUnknownOption(true)
  .action(async (options, command) => {
    try {
      const args = command.args || [];
      const passthrough: string[] = [];
      if (options.config) passthrough.push('--config', options.config);
      passthrough.push(...args);
      await serverStart(passthrough, {
        verbose: options.verbose,
        production: options.production,
        detach: options.detach,
        port: options.port,
        pgwirePort: options.pgwirePort,
      });
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

serverCmd
  .command('stop')
  .description('Stop a running RaisinDB server')
  .action(async () => {
    try {
      await serverStop();
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

serverCmd
  .command('status')
  .description('Check server health and status')
  .action(async () => {
    try {
      await serverStatus();
      process.exit(0);
    } catch (error) {
      console.error('Error:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

serverCmd
  .command('logs')
  .description('View server logs')
  .option('-f, --follow', 'Stream logs in real-time')
  .option('-n, --lines <count>', 'Number of lines to show', '50')
  .action(async (options) => {
    try {
      await serverLogs({ follow: options.follow, lines: options.lines });
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

/** Run an async admin command action with uniform error handling. */
async function runAdmin(action: () => Promise<void>): Promise<never> {
  try {
    await action();
    process.exit(0);
  } catch (error) {
    console.error('Error:', error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}

/** Commander collector for repeatable options (e.g. --model). */
function collect(value: string, previous: string[]): string[] {
  return [...previous, value];
}

// Repository administration commands (operate over the HTTP API + token,
// work identically against local and remote servers)
const repoCmd = program
  .command('repo')
  .description('Repository administration (create/list/delete via server API)');

repoCmd
  .command('create <name>')
  .description('Create a repository')
  .option('-d, --description <text>', 'Repository description')
  .option('--exists-ok', 'Succeed if the repository already exists')
  .action((name, options) =>
    runAdmin(() => repoCreate(name, { description: options.description, existsOk: options.existsOk }))
  );

repoCmd
  .command('list')
  .description('List repositories')
  .option('--json', 'Machine-readable JSON output')
  .action((options) => runAdmin(() => repoList({ json: options.json })));

repoCmd
  .command('delete <name>')
  .description('Delete a repository (irreversible, requires --yes)')
  .option('-y, --yes', 'Confirm deletion')
  .action((name, options) => runAdmin(() => repoDelete(name, { yes: options.yes })));

// AI provider configuration commands (tenant-level secrets, gh-secret style)
const aiCmd = program
  .command('ai')
  .description('Tenant AI configuration');

const aiProviderCmd = aiCmd
  .command('provider')
  .description('Manage tenant AI providers (API keys are never echoed)');

aiProviderCmd
  .command('set <slug>')
  .description('Create or update a provider by slug (read-modify-write; other providers and stored keys are preserved)')
  .option('--kind <kind>', 'Provider kind (openai, anthropic, custom, ...); required when creating a new slug')
  .option('--api-key <value>', 'API key value (prefer --api-key-stdin or --api-key-env in CI)')
  .option('--api-key-stdin', 'Read the API key from stdin')
  .option('--api-key-env <var>', 'Read the API key from an environment variable')
  .option('--endpoint <url>', 'Custom API endpoint')
  .option('--display-name <name>', 'Human-readable name shown in UIs')
  .option('--icon-url <url>', 'Icon URL shown in UIs')
  .option('--enabled', 'Enable the provider')
  .option('--disabled', 'Disable the provider')
  .option('-m, --model <spec>', 'Model as model_id[:display_name] (repeatable; first becomes default)', collect, [])
  .option('--tenant <tenant>', 'Tenant ID', 'default')
  .action((slug, options) =>
    runAdmin(() =>
      aiProviderSet(slug, {
        kind: options.kind,
        apiKey: options.apiKey,
        apiKeyStdin: options.apiKeyStdin,
        apiKeyEnv: options.apiKeyEnv,
        endpoint: options.endpoint,
        displayName: options.displayName,
        iconUrl: options.iconUrl,
        enabled: options.enabled,
        disabled: options.disabled,
        model: options.model,
        tenant: options.tenant,
      })
    )
  );

aiProviderCmd
  .command('list')
  .description('List configured providers (slug, kind, endpoint, enabled, has_api_key, model count)')
  .option('--tenant <tenant>', 'Tenant ID', 'default')
  .option('--json', 'Machine-readable JSON output')
  .action((options) => runAdmin(() => aiProviderList({ tenant: options.tenant, json: options.json })));

aiProviderCmd
  .command('test <slug>')
  .description('Test the live connection to a configured provider (by slug)')
  .option('--tenant <tenant>', 'Tenant ID', 'default')
  .action((slug, options) => runAdmin(() => aiProviderTest(slug, { tenant: options.tenant })));

// Encrypted secret store (gh-secret style: the value is never echoed).
// There is deliberately no `secret get` — the API exposes no plaintext read,
// and a CLI that printed one would leak credentials into scrollback and CI
// logs. Functions read values via raisin.secrets.get, gated by secret_policy.
const secretCmd = program
  .command('secret')
  .description('Manage encrypted secrets (branch-scoped; values are never echoed)');

secretCmd
  .command('set <name>')
  .description('Write a new version of a secret (value read from STDIN by default)')
  .option('--value <value>', 'Value inline (discouraged: lands in shell history)')
  .option('--value-env <var>', 'Read the value from an environment variable')
  .option('-r, --repo <name>', 'Target repository (defaults to the configured repo)')
  .option('-b, --branch <branch>', 'Target branch', 'main')
  .action((name, options) =>
    runAdmin(() =>
      secretSet(name, {
        value: options.value,
        valueEnv: options.valueEnv,
        repo: options.repo,
        branch: options.branch,
      })
    )
  );

secretCmd
  .command('list')
  .description('List secrets (metadata only — never values)')
  .option('-r, --repo <name>', 'Target repository (defaults to the configured repo)')
  .option('-b, --branch <branch>', 'Target branch', 'main')
  .option('--json', 'Machine-readable JSON output')
  .action((options) =>
    runAdmin(() => secretList({ repo: options.repo, branch: options.branch, json: options.json }))
  );

secretCmd
  .command('show <name>')
  .description('Show one secret\'s metadata (version, timestamps, author) — never its value')
  .option('-r, --repo <name>', 'Target repository (defaults to the configured repo)')
  .option('-b, --branch <branch>', 'Target branch', 'main')
  .option('--json', 'Machine-readable JSON output')
  .action((name, options) =>
    runAdmin(() =>
      secretShow(name, { repo: options.repo, branch: options.branch, json: options.json })
    )
  );

secretCmd
  .command('rotate <name>')
  .description('Append a new version stamped as a rotation (value from STDIN by default)')
  .option('--value <value>', 'Value inline (discouraged: lands in shell history)')
  .option('--value-env <var>', 'Read the value from an environment variable')
  .option('-r, --repo <name>', 'Target repository (defaults to the configured repo)')
  .option('-b, --branch <branch>', 'Target branch', 'main')
  .action((name, options) =>
    runAdmin(() =>
      secretRotate(name, {
        value: options.value,
        valueEnv: options.valueEnv,
        repo: options.repo,
        branch: options.branch,
      })
    )
  );

secretCmd
  .command('rm <name>')
  .description('Delete a secret (tombstone; requires --yes)')
  .option('-r, --repo <name>', 'Target repository (defaults to the configured repo)')
  .option('-b, --branch <branch>', 'Target branch', 'main')
  .option('-y, --yes', 'Confirm deletion')
  .action((name, options) =>
    runAdmin(() =>
      secretRemove(name, { repo: options.repo, branch: options.branch, yes: options.yes })
    )
  );

// Identity user commands
const userCmd = program
  .command('user')
  .description('Identity user administration');

userCmd
  .command('register <email>')
  .description('Register an identity user (demo/login user) for a repository')
  .option('-p, --password <password>', 'Password (prefer --password-stdin in CI)')
  .option('--password-stdin', 'Read the password from stdin')
  .option('-r, --repo <name>', 'Target repository (required)')
  .option('--display-name <name>', 'Display name')
  .option('--tenant <tenant>', 'Tenant ID', 'default')
  .option('--exists-ok', 'Succeed if the user already exists')
  .action((email, options) =>
    runAdmin(() =>
      userRegister(email, {
        password: options.password,
        passwordStdin: options.passwordStdin,
        repo: options.repo,
        displayName: options.displayName,
        tenant: options.tenant,
        existsOk: options.existsOk,
      })
    )
  );

// CORS allowed-origins commands (repo-level by default, --tenant-level for
// the tenant-wide fallback used when a repo has no own config)
const corsCmd = program
  .command('cors')
  .description('Manage CORS allowed origins (repo-level, or --tenant-level)');

corsCmd
  .command('add <origin>')
  .description('Add an origin to the CORS allow-list')
  .option('-r, --repo <name>', 'Target repository')
  .option('--tenant-level', 'Operate on the tenant-level config instead of a repo')
  .option('--tenant <tenant>', 'Tenant ID', 'default')
  .action((origin, options) =>
    runAdmin(() => corsAdd(origin, { repo: options.repo, tenantLevel: options.tenantLevel, tenant: options.tenant }))
  );

corsCmd
  .command('list')
  .description('List CORS allowed origins')
  .option('-r, --repo <name>', 'Target repository')
  .option('--tenant-level', 'Operate on the tenant-level config instead of a repo')
  .option('--tenant <tenant>', 'Tenant ID', 'default')
  .option('--json', 'Machine-readable JSON output')
  .action((options) =>
    runAdmin(() =>
      corsList({ repo: options.repo, tenantLevel: options.tenantLevel, tenant: options.tenant, json: options.json })
    )
  );

corsCmd
  .command('remove <origin>')
  .description('Remove an origin from the CORS allow-list')
  .option('-r, --repo <name>', 'Target repository')
  .option('--tenant-level', 'Operate on the tenant-level config instead of a repo')
  .option('--tenant <tenant>', 'Tenant ID', 'default')
  .action((origin, options) =>
    runAdmin(() => corsRemove(origin, { repo: options.repo, tenantLevel: options.tenantLevel, tenant: options.tenant }))
  );

// Authentication commands
program
  .command('login')
  .description('Authenticate with a RaisinDB server (browser flow, or non-interactive with --username/--password or --token)')
  .option('-s, --server <url>', 'Server URL (default: http://localhost:8080)')
  .option('-u, --username <username>', 'Username for non-interactive login')
  .option('-p, --password <password>', 'Password for non-interactive login')
  .option('-t, --token <token>', 'Store an existing token directly (non-interactive)')
  .option('--tenant <tenant>', 'Tenant for username/password login', 'default')
  .action(async (options) => {
    const serverUrl = options.server || getServer() || 'http://localhost:8080';
    try {
      if (options.token) {
        // Non-interactive: store token directly
        loginWithToken(serverUrl, options.token);
        console.log(`Token stored for ${serverUrl} in .raisinrc`);
        process.exit(0);
      }

      if (options.username || options.password) {
        // Non-interactive: username/password against system auth
        if (!options.username || !options.password) {
          console.error('Both --username and --password are required for non-interactive login.');
          process.exit(1);
        }
        const result = await loginWithPassword(
          serverUrl,
          options.username,
          options.password,
          options.tenant
        );
        console.log(`Authenticated with ${serverUrl} as ${result.username || options.username} (tenant: ${options.tenant}).`);
        if (result.expiresAt) {
          console.log(`Token expires at ${new Date(result.expiresAt * 1000).toISOString()}`);
        }
        console.log('Token saved to .raisinrc');
        process.exit(0);
      }

      // Interactive: browser flow
      console.log(`\nAuthenticating with ${serverUrl}...`);
      console.log('Opening browser for login...\n');
      await login(serverUrl);
      console.log('\nAuthenticated successfully. Token saved to .raisinrc');
      console.log('All subsequent commands will use this authentication.\n');
      process.exit(0);
    } catch (error) {
      console.error('Login failed:', error instanceof Error ? error.message : String(error));
      process.exit(1);
    }
  });

program
  .command('logout')
  .description('Clear stored authentication token')
  .action(() => {
    logout();
    console.log('Logged out. Token cleared from .raisinrc');
    process.exit(0);
  });

// Interactive shell mode (explicit command)
program
  .command('shell')
  .description('Start interactive SQL shell')
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

// Show branded help when no command given
if (process.argv.length <= 2) {
  const { unmount } = render(<HelpDisplay version={pkg.version} />);
  setTimeout(() => { unmount(); process.exit(0); }, 100);
} else {
  program.parse();
}
