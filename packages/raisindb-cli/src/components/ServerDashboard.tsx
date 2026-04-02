import React from 'react';
import { Box, Text } from 'ink';
import Gradient from 'ink-gradient';
import InkSpinner from 'ink-spinner';

const BRAND_COLORS = ['#B8754E', '#D97706', '#EA580C', '#f97316'];

export type ServerPhase = 'installing' | 'starting' | 'ready' | 'error' | 'stopped';

export interface ServerDashboardState {
  phase: ServerPhase;
  version?: string;
  devMode: boolean;
  httpPort: number;
  pgwirePort: number;
  adminUser?: string;
  adminPassword?: string;
  dataDir?: string;
  error?: string;
  lastLog?: string;
}

function ConnectionInfo({ state }: { state: ServerDashboardState }) {
  const { httpPort, pgwirePort } = state;
  return (
    <Box flexDirection="column" paddingX={2} paddingY={1}>
      <Box>
        <Box width={16}><Text dimColor>HTTP API</Text></Box>
        <Text color="cyan">http://localhost:{httpPort}</Text>
      </Box>
      <Box>
        <Box width={16}><Text dimColor>PostgreSQL</Text></Box>
        <Text color="cyan">postgresql://localhost:{pgwirePort}</Text>
      </Box>
      <Box>
        <Box width={16}><Text dimColor>Admin UI</Text></Box>
        <Text color="cyan">http://localhost:{httpPort}/admin</Text>
      </Box>
    </Box>
  );
}

function Credentials({ state }: { state: ServerDashboardState }) {
  if (!state.adminUser) return null;
  return (
    <Box flexDirection="column" paddingX={2} paddingY={1}>
      <Box>
        <Box width={16}><Text dimColor>Username</Text></Box>
        <Text bold color="white">{state.adminUser}</Text>
      </Box>
      <Box>
        <Box width={16}><Text dimColor>Password</Text></Box>
        <Text bold color="white">{state.adminPassword || 'admin'}</Text>
      </Box>
    </Box>
  );
}

function DevModeWarning() {
  return (
    <Box paddingX={2} paddingY={1}>
      <Text color="yellow">{'⚠'} Dev mode: insecure defaults. Use </Text>
      <Text color="white" bold>--production</Text>
      <Text color="yellow"> for secure config.</Text>
    </Box>
  );
}

export function ServerDashboard({ state }: { state: ServerDashboardState }) {
  const { phase, version, devMode, error } = state;

  return (
    <Box flexDirection="column" borderStyle="round" borderColor="#B8754E" paddingX={0} paddingY={0}>
      {/* Header */}
      <Box paddingX={2} paddingTop={1}>
        <Gradient colors={BRAND_COLORS}>
          <Text bold> RaisinDB </Text>
        </Gradient>
        {version && <Text dimColor> v{version}</Text>}
        <Text>  </Text>
        {devMode && <Text color="yellow" bold>Development Mode</Text>}
        {!devMode && <Text color="green" bold>Production Mode</Text>}
      </Box>

      {/* Starting phase */}
      {phase === 'starting' && (
        <Box paddingX={2} paddingY={1}>
          <Text color="cyan"><InkSpinner type="dots" /></Text>
          <Text> Starting server...</Text>
          {state.lastLog && (
            <Text dimColor>  {state.lastLog}</Text>
          )}
        </Box>
      )}

      {/* Ready phase */}
      {phase === 'ready' && (
        <>
          <ConnectionInfo state={state} />
          <Credentials state={state} />
          {devMode && <DevModeWarning />}
          <Box paddingX={2} paddingBottom={1}>
            <Text color="green">{'✓'} Server ready</Text>
            <Text dimColor>  Press Ctrl+C to stop</Text>
          </Box>
        </>
      )}

      {/* Error phase */}
      {phase === 'error' && (
        <Box flexDirection="column" paddingX={2} paddingY={1}>
          <Box>
            <Text color="red">{'✗'} Server failed to start</Text>
          </Box>
          {error && (
            <Box paddingTop={1}>
              <Text color="red">{error}</Text>
            </Box>
          )}
        </Box>
      )}

      {/* Stopped */}
      {phase === 'stopped' && (
        <Box paddingX={2} paddingY={1}>
          <Text dimColor>Server stopped</Text>
        </Box>
      )}
    </Box>
  );
}
