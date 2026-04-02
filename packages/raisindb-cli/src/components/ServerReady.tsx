import React, { useState, useEffect } from 'react';
import { Box, Text } from 'ink';
import Gradient from 'ink-gradient';
import BigText from 'ink-big-text';

const BRAND_COLORS = ['#B8754E', '#D97706', '#EA580C', '#f97316'];

interface ServerReadyProps {
  version?: string;
  devMode: boolean;
  httpPort: string;
  pgwirePort: string;
  adminPassword?: string | null;
  dataDir?: string;
  isFirstRun: boolean;
  pid: number;
}

export function ServerReady({
  version, devMode, httpPort, pgwirePort,
  adminPassword, dataDir, isFirstRun, pid
}: ServerReadyProps) {
  const [showGetStarted, setShowGetStarted] = useState(false);

  useEffect(() => {
    if (isFirstRun) {
      const timer = setTimeout(() => setShowGetStarted(true), 800);
      return () => clearTimeout(timer);
    }
    setShowGetStarted(true);
  }, [isFirstRun]);

  return (
    <Box flexDirection="column" paddingX={1} paddingY={1}>
      {/* Logo */}
      {isFirstRun && (
        <Box marginBottom={1}>
          <Gradient colors={BRAND_COLORS}>
            <BigText text="RaisinDB" font="simple" />
          </Gradient>
        </Box>
      )}

      {/* Header */}
      {!isFirstRun && (
        <Box marginBottom={1}>
          <Gradient colors={BRAND_COLORS}>
            <Text bold> RaisinDB </Text>
          </Gradient>
          {version && <Text dimColor> v{version}</Text>}
          <Text>  </Text>
          {devMode && <Text color="yellow" bold>Development Mode</Text>}
          {!devMode && <Text color="green" bold>Production</Text>}
        </Box>
      )}

      {isFirstRun && (
        <Box marginBottom={1}>
          {version && <Text dimColor>v{version}</Text>}
          <Text>  </Text>
          {devMode && <Text color="yellow" bold>Development Mode</Text>}
        </Box>
      )}

      {/* Connection info */}
      <Box flexDirection="column" marginBottom={1}>
        <Box>
          <Box width={15}><Text dimColor>HTTP API</Text></Box>
          <Text color="cyan">http://localhost:{httpPort}</Text>
        </Box>
        <Box>
          <Box width={15}><Text dimColor>PostgreSQL</Text></Box>
          <Text color="cyan">postgresql://localhost:{pgwirePort}</Text>
        </Box>
        <Box>
          <Box width={15}><Text dimColor>Admin UI</Text></Box>
          <Text color="cyan">http://localhost:{httpPort}/admin</Text>
        </Box>
        {dataDir && (
          <Box>
            <Box width={15}><Text dimColor>Data</Text></Box>
            <Text dimColor>{dataDir}</Text>
          </Box>
        )}
      </Box>

      {/* Credentials (first run only) */}
      {adminPassword && (
        <Box flexDirection="column" marginBottom={1} borderStyle="round" borderColor="yellow" paddingX={2} paddingY={1}>
          <Text bold color="yellow">Admin Credentials</Text>
          <Box marginTop={1}>
            <Box width={15}><Text dimColor>Username</Text></Box>
            <Text bold>admin</Text>
          </Box>
          <Box>
            <Box width={15}><Text dimColor>Password</Text></Box>
            <Text bold>{adminPassword}</Text>
          </Box>
          <Box marginTop={1}>
            <Text color="yellow">Save this password — it won't be shown again.</Text>
          </Box>
        </Box>
      )}

      {/* Dev mode warning */}
      {devMode && (
        <Box marginBottom={1}>
          <Text color="yellow">! </Text>
          <Text dimColor>Dev mode — insecure defaults. Use </Text>
          <Text bold>--production</Text>
          <Text dimColor> for secure config.</Text>
        </Box>
      )}

      {/* Status */}
      <Box marginBottom={1}>
        <Text color="green">✓ </Text>
        <Text>Server ready </Text>
        <Text dimColor>(PID {pid})</Text>
      </Box>

      {/* Get started (first run) */}
      {isFirstRun && showGetStarted && (
        <Box flexDirection="column" marginBottom={1}>
          <Text bold dimColor>Get started:</Text>
          <Box paddingLeft={2} flexDirection="column">
            <Box>
              <Text dimColor>$ </Text>
              <Text>psql -h localhost -p {pgwirePort} -U admin</Text>
            </Box>
            <Box>
              <Text dimColor>$ </Text>
              <Text>open </Text>
              <Text color="cyan">http://localhost:{httpPort}/admin</Text>
            </Box>
            <Box>
              <Text dimColor>$ </Text>
              <Text>raisindb shell</Text>
            </Box>
          </Box>
        </Box>
      )}

      {/* Footer */}
      <Box>
        <Text dimColor>Stop: raisindb server stop  |  Logs: raisindb server logs -f</Text>
      </Box>
    </Box>
  );
}
