import React from 'react';
import { Box, Text } from 'ink';
import Gradient from 'ink-gradient';

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
  readyTime?: string;
}

function Arrow() {
  return <Text color="#D97706">{'➜'}</Text>;
}

export function ServerReady({
  version, devMode, httpPort, pgwirePort,
  adminPassword, dataDir, pid, readyTime
}: ServerReadyProps) {

  return (
    <Box flexDirection="column" paddingX={1} paddingY={1}>
      {/* Header */}
      <Box>
        <Gradient colors={BRAND_COLORS}>
          <Text bold>RaisinDB</Text>
        </Gradient>
        {version && <Text dimColor> v{version}</Text>}
        <Text>  </Text>
        {devMode && <Text color="yellow">Development Mode</Text>}
        {!devMode && <Text color="green">Production</Text>}
        {readyTime && <Text dimColor>  ready in {readyTime}</Text>}
      </Box>

      {/* Connection info */}
      <Box flexDirection="column" marginTop={1}>
        <Box><Text>  </Text><Arrow /><Text>  </Text><Box width={10}><Text dimColor>HTTP</Text></Box><Text color="cyan">http://localhost:{httpPort}</Text></Box>
        <Box><Text>  </Text><Arrow /><Text>  </Text><Box width={10}><Text dimColor>PgSQL</Text></Box><Text color="cyan">postgresql://localhost:{pgwirePort}</Text></Box>
        <Box><Text>  </Text><Arrow /><Text>  </Text><Box width={10}><Text dimColor>Admin</Text></Box><Text color="cyan">http://localhost:{httpPort}/admin</Text></Box>
        {dataDir && (
          <Box><Text>  </Text><Arrow /><Text>  </Text><Box width={10}><Text dimColor>Data</Text></Box><Text dimColor>{dataDir}</Text></Box>
        )}
      </Box>

      {/* Credentials (first run only) */}
      {adminPassword && (
        <Box flexDirection="column" marginTop={1} borderStyle="round" borderColor="yellow" paddingX={2} paddingY={1}>
          <Box><Box width={14}><Text dimColor>Username</Text></Box><Text bold color="white">admin</Text></Box>
          <Box><Box width={14}><Text dimColor>Password</Text></Box><Text bold color="white">{adminPassword}</Text></Box>
          <Box marginTop={1}>
            <Text color="yellow">Save this password — it won't be shown again.</Text>
          </Box>
        </Box>
      )}

      {/* Dev mode warning */}
      {devMode && (
        <Box marginTop={1}>
          <Text color="yellow">  ! </Text>
          <Text dimColor>Dev mode — insecure defaults. Use </Text>
          <Text bold>--production</Text>
          <Text dimColor> for secure config.</Text>
        </Box>
      )}

      {/* Status */}
      <Box marginTop={1}>
        <Text color="green">  ✓ </Text>
        <Text>Server ready </Text>
        <Text dimColor>(PID {pid})</Text>
      </Box>

      <Box marginTop={0}>
        <Text dimColor>  Stop: raisindb server stop  |  Logs: raisindb server logs -f</Text>
      </Box>
    </Box>
  );
}
