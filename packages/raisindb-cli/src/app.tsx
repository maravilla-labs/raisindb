import React, { useState, useEffect } from 'react';
import { Box } from 'ink';
import fs from 'fs';
import path from 'path';
import Banner from './components/Banner.js';
import Shell from './components/Shell.js';
import SqlShell from './components/SqlShell.js';
import PackageCreator from './components/PackageCreator.js';
import LoginScreen from './components/LoginScreen.js';
import { loadConfig, saveConfig, Config } from './config.js';
import { logout, cancelLogin } from './auth.js';
import {
  listRepositories,
  listPackages,
  installPackage as apiInstallPackage,
  uploadPackage as apiUploadPackage,
  getAuthProviders,
  getAuthSessions,
  revokeSession,
  getCurrentUser,
} from './api.js';

export interface AppProps {
  serverUrl?: string;
  database?: string;
}

export type AppMode = 'shell' | 'sql' | 'package-create' | 'login';

export interface AppState {
  config: Config;
  mode: AppMode;
  currentDatabase: string | null;
  connected: boolean;
}

const App: React.FC<AppProps> = ({ serverUrl, database }) => {
  const [state, setState] = useState<AppState>({
    config: { server: serverUrl ?? null, token: null, default_repo: null },
    mode: 'shell',
    currentDatabase: database || null,
    connected: false,
  });

  useEffect(() => {
    // Load config on startup
    const loadedConfig = loadConfig();
    setState((prev) => ({
      ...prev,
      config: {
        ...loadedConfig,
        server: serverUrl || loadedConfig.server,
      },
      currentDatabase: database || loadedConfig.default_repo || null,
    }));
  }, [serverUrl, database]);

  const handleCommand = async (command: string, args: string[]) => {
    const cmd = command.toLowerCase();

    switch (cmd) {
      case '/help':
        // Help is handled in Shell component
        break;

      case '/connect':
        if (args.length === 0) {
          return { type: 'error', message: 'Usage: /connect <url>' };
        }
        const newServer = args[0];
        const updatedConfig = { ...state.config, server: newServer };
        saveConfig(updatedConfig);
        setState((prev) => ({ ...prev, config: updatedConfig, connected: true }));
        return { type: 'success', message: `Connected to ${newServer}` };

      case '/login':
        setState((prev) => ({ ...prev, mode: 'login' }));
        return null; // Don't show message, switch to login mode

      case '/logout':
        logout();
        const logoutConfig = { ...state.config, token: null };
        saveConfig(logoutConfig);
        setState((prev) => ({ ...prev, config: logoutConfig }));
        return { type: 'success', message: 'Logged out successfully' };

      case 'use':
        if (args.length === 0) {
          return { type: 'error', message: 'Usage: use <database>' };
        }
        const dbName = args[0];
        setState((prev) => ({ ...prev, currentDatabase: dbName }));
        const dbConfig = { ...state.config, default_repo: dbName };
        saveConfig(dbConfig);
        return { type: 'success', message: `Switched to database: ${dbName}` };

      case '/databases':
      case '/repos':
        try {
          const repos = await listRepositories();
          if (repos.length === 0) {
            return { type: 'info', message: 'No repositories found' };
          }
          const repoList = repos.map(r => {
            const desc = r.config?.description ? ` - ${r.config.description}` : '';
            return `  • ${r.repo_id}${desc}`;
          }).join('\n');
          return { type: 'info', message: `Repositories:\n${repoList}` };
        } catch (error) {
          return { type: 'error', message: `Failed to list repositories: ${error instanceof Error ? error.message : String(error)}` };
        }

      case '/sql':
        if (!state.currentDatabase) {
          return { type: 'error', message: 'No database selected. Use "use <database>" first.' };
        }
        setState((prev) => ({ ...prev, mode: 'sql' }));
        return null; // Don't show message, just switch mode

      case '/exit-sql':
        setState((prev) => ({ ...prev, mode: 'shell' }));
        return { type: 'success', message: 'Exited SQL mode' };

      case '/packages':
        if (!state.currentDatabase) {
          return { type: 'error', message: 'No database selected. Use "use <database>" first.' };
        }
        try {
          const packages = await listPackages(state.currentDatabase);
          if (packages.length === 0) {
            return { type: 'info', message: 'No packages found' };
          }
          const pkgList = packages.map(p => {
            const status = p.installed ? '✓' : '○';
            return `  ${status} ${p.name} v${p.version}`;
          }).join('\n');
          return { type: 'info', message: `Packages:\n${pkgList}` };
        } catch (error) {
          return { type: 'error', message: `Failed to list packages: ${error instanceof Error ? error.message : String(error)}` };
        }

      case '/install':
        if (args.length === 0) {
          return { type: 'error', message: 'Usage: /install <package-name>' };
        }
        if (!state.currentDatabase) {
          return { type: 'error', message: 'No database selected. Use "use <database>" first.' };
        }
        try {
          await apiInstallPackage(state.currentDatabase, args[0]);
          return { type: 'success', message: `Package ${args[0]} installed successfully` };
        } catch (error) {
          return { type: 'error', message: `Failed to install package: ${error instanceof Error ? error.message : String(error)}` };
        }

      case '/upload':
        const uploadFile = args[0] || '';
        if (!uploadFile) {
          return { type: 'error', message: 'Usage: /upload <file.rap>' };
        }
        if (!state.currentDatabase) {
          return { type: 'error', message: 'No database selected. Use "use <database>" first.' };
        }
        try {
          const resolvedFile = path.resolve(uploadFile);
          if (!fs.existsSync(resolvedFile)) {
            return { type: 'error', message: `File not found: ${resolvedFile}` };
          }
          if (!resolvedFile.endsWith('.rap')) {
            return { type: 'error', message: 'File must have .rap extension' };
          }

          const fileName = path.basename(resolvedFile);
          const fileContent = fs.readFileSync(resolvedFile);

          const result = await apiUploadPackage(state.currentDatabase, fileContent, fileName);
          return { type: 'success', message: `Package uploaded: ${result.name} v${result.version}` };
        } catch (error) {
          return { type: 'error', message: `Failed to upload: ${error instanceof Error ? error.message : String(error)}` };
        }

      case '/create':
      case '/package':
        if (args[0] === 'create' || cmd === '/create') {
          setState((prev) => ({ ...prev, mode: 'package-create' }));
          return null; // Don't show message, just switch mode
        }
        return { type: 'error', message: 'Usage: /package create or /create' };

      case '/clear':
        // This is handled by the Shell component
        break;

      case '/status':
        const serverStatus = state.config.server || 'not configured';
        const authStatus = state.config.token ? 'authenticated' : 'not authenticated';
        const dbStatus = state.currentDatabase || 'none';
        return {
          type: 'info',
          message: `Server: ${serverStatus}\nAuth: ${authStatus}\nDatabase: ${dbStatus}`,
        };

      // Authentication commands
      case '/auth':
        if (args.length === 0) {
          return { type: 'info', message: 'Usage: /auth <providers|sessions|me|revoke <session_id>>' };
        }
        const authSubcmd = args[0].toLowerCase();

        if (authSubcmd === 'providers') {
          try {
            const providersResponse = await getAuthProviders();
            const providerList = providersResponse.providers.length > 0
              ? providersResponse.providers.map(p => `  • ${p.display_name} (${p.id})`).join('\n')
              : '  (none configured)';
            const authMethods = [];
            if (providersResponse.local_enabled) authMethods.push('Password');
            if (providersResponse.magic_link_enabled) authMethods.push('Magic Link');
            return {
              type: 'info',
              message: `Auth Providers:\n${providerList}\n\nEnabled methods: ${authMethods.join(', ') || 'none'}`,
            };
          } catch (error) {
            return { type: 'error', message: `Failed to get providers: ${error instanceof Error ? error.message : String(error)}` };
          }
        }

        if (authSubcmd === 'sessions') {
          try {
            const sessionsResponse = await getAuthSessions();
            if (sessionsResponse.sessions.length === 0) {
              return { type: 'info', message: 'No active sessions' };
            }
            const sessionList = sessionsResponse.sessions.map(s => {
              const current = s.is_current ? ' (current)' : '';
              const ua = s.user_agent ? ` - ${s.user_agent.substring(0, 30)}...` : '';
              return `  ${s.is_current ? '✓' : '○'} ${s.id.substring(0, 8)}... via ${s.auth_strategy}${ua}${current}`;
            }).join('\n');
            return { type: 'info', message: `Active Sessions:\n${sessionList}` };
          } catch (error) {
            return { type: 'error', message: `Failed to get sessions: ${error instanceof Error ? error.message : String(error)}` };
          }
        }

        if (authSubcmd === 'me') {
          try {
            const user = await getCurrentUser();
            const providers = user.linked_providers.length > 0
              ? user.linked_providers.join(', ')
              : 'none';
            const verified = user.email_verified ? '✓' : '○';
            return {
              type: 'info',
              message: `Identity: ${user.display_name || user.email}\nEmail: ${user.email} ${verified}\nID: ${user.id}\nLinked providers: ${providers}`,
            };
          } catch (error) {
            return { type: 'error', message: `Failed to get user info: ${error instanceof Error ? error.message : String(error)}` };
          }
        }

        if (authSubcmd === 'revoke') {
          if (args.length < 2) {
            return { type: 'error', message: 'Usage: /auth revoke <session_id>' };
          }
          try {
            await revokeSession(args[1]);
            return { type: 'success', message: `Session ${args[1]} revoked` };
          } catch (error) {
            return { type: 'error', message: `Failed to revoke session: ${error instanceof Error ? error.message : String(error)}` };
          }
        }

        return { type: 'error', message: `Unknown auth command: ${authSubcmd}. Use: providers, sessions, me, revoke` };

      case '/quit':
      case '/exit':
        process.exit(0);

      default:
        return { type: 'error', message: `Unknown command: ${command}. Type /help for available commands.` };
    }
  };

  return (
    <Box flexDirection="column">
      <Banner />
      {state.mode === 'shell' && (
        <Shell
          currentDatabase={state.currentDatabase}
          onCommand={handleCommand}
        />
      )}
      {state.mode === 'sql' && (
        <SqlShell
          currentDatabase={state.currentDatabase}
          onExit={() => setState((prev) => ({ ...prev, mode: 'shell' }))}
        />
      )}
      {state.mode === 'package-create' && (
        <PackageCreator
          onExit={() => setState((prev) => ({ ...prev, mode: 'shell' }))}
          onSuccess={(packagePath) => {
            console.log(`Package created: ${packagePath}`);
          }}
        />
      )}
      {state.mode === 'login' && (
        <LoginScreen
          serverUrl={state.config.server || 'http://localhost:8081'}
          onSuccess={(token) => {
            const loginConfig = { ...state.config, token };
            saveConfig(loginConfig);
            setState((prev) => ({ ...prev, config: loginConfig, mode: 'shell' }));
          }}
          onCancel={() => {
            cancelLogin();
            setState((prev) => ({ ...prev, mode: 'shell' }));
          }}
          onError={() => {
            setState((prev) => ({ ...prev, mode: 'shell' }));
          }}
        />
      )}
    </Box>
  );
};

export default App;
