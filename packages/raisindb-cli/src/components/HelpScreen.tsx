import React from 'react';
import { Box, Text } from 'ink';

const HelpScreen: React.FC = () => {
  return (
    <Box flexDirection="column" paddingX={1}>
      <Box marginTop={1}>
        <Text bold color="cyan">
          RaisinDB CLI - Available Commands
        </Text>
      </Box>

      <Box flexDirection="column" marginTop={1}>
        <Text bold color="yellow">
          Connection & Authentication:
        </Text>
        <Box flexDirection="column" marginLeft={2}>
          <Text>
            <Text color="green">/connect {'<url>'}</Text>
            <Text dimColor>         Connect to server</Text>
          </Text>
          <Text>
            <Text color="green">/login</Text>
            <Text dimColor>                 Browser-based authentication</Text>
          </Text>
          <Text>
            <Text color="green">/logout</Text>
            <Text dimColor>                Clear authentication</Text>
          </Text>
          <Text>
            <Text color="green">/auth providers</Text>
            <Text dimColor>        List configured auth providers</Text>
          </Text>
          <Text>
            <Text color="green">/auth sessions</Text>
            <Text dimColor>         List your active sessions</Text>
          </Text>
          <Text>
            <Text color="green">/auth me</Text>
            <Text dimColor>               Show current identity info</Text>
          </Text>
          <Text>
            <Text color="green">/auth revoke {'<id>'}</Text>
            <Text dimColor>    Revoke a session by ID</Text>
          </Text>
        </Box>
      </Box>

      <Box flexDirection="column" marginTop={1}>
        <Text bold color="yellow">
          Database Operations:
        </Text>
        <Box flexDirection="column" marginLeft={2}>
          <Text>
            <Text color="green">use {'<database>'}</Text>
            <Text dimColor>          Switch database</Text>
          </Text>
          <Text>
            <Text color="green">/databases</Text>
            <Text dimColor>              List available databases</Text>
          </Text>
        </Box>
      </Box>

      <Box flexDirection="column" marginTop={1}>
        <Text bold color="yellow">
          Navigation (shell-like):
        </Text>
        <Box flexDirection="column" marginLeft={2}>
          <Text>
            <Text color="green">ls [-l]</Text>
            <Text dimColor>                List nodes in current path</Text>
          </Text>
          <Text>
            <Text color="green">cd {'<path>'}</Text>
            <Text dimColor>             Navigate to node (.. for parent)</Text>
          </Text>
          <Text>
            <Text color="green">lstree [depth]</Text>
            <Text dimColor>         Show tree view (default depth: 3)</Text>
          </Text>
          <Text>
            <Text color="green">pwd</Text>
            <Text dimColor>                    Show current path</Text>
          </Text>
          <Text>
            <Text color="green">cat {'<node>'}</Text>
            <Text dimColor>            Show node JSON (syntax highlighted)</Text>
          </Text>
        </Box>
      </Box>

      <Box flexDirection="column" marginTop={1}>
        <Text bold color="yellow">
          SQL Mode:
        </Text>
        <Box flexDirection="column" marginLeft={2}>
          <Text>
            <Text color="green">/sql</Text>
            <Text dimColor>                   Enter SQL mode</Text>
          </Text>
          <Text>
            <Text color="green">/exit-sql</Text>
            <Text dimColor>              Exit SQL mode</Text>
          </Text>
        </Box>
      </Box>

      <Box flexDirection="column" marginTop={1}>
        <Text bold color="yellow">
          Package Management:
        </Text>
        <Box flexDirection="column" marginLeft={2}>
          <Text>
            <Text color="green">/packages</Text>
            <Text dimColor>              List installed packages</Text>
          </Text>
          <Text>
            <Text color="green">/install {'<name>'}</Text>
            <Text dimColor>        Install a package</Text>
          </Text>
          <Text>
            <Text color="green">/create [path]</Text>
            <Text dimColor>         Create package (Tab completes paths)</Text>
          </Text>
          <Text>
            <Text color="green">/upload [file]</Text>
            <Text dimColor>         Upload package (Tab completes paths)</Text>
          </Text>
        </Box>
      </Box>

      <Box flexDirection="column" marginTop={1}>
        <Text bold color="yellow">
          Other Commands:
        </Text>
        <Box flexDirection="column" marginLeft={2}>
          <Text>
            <Text color="green">/status</Text>
            <Text dimColor>                Show connection status</Text>
          </Text>
          <Text>
            <Text color="green">/help</Text>
            <Text dimColor>                  Show this help message</Text>
          </Text>
          <Text>
            <Text color="green">/clear</Text>
            <Text dimColor>                 Clear screen</Text>
          </Text>
          <Text>
            <Text color="green">/quit</Text>
            <Text dimColor>                  Exit the CLI</Text>
          </Text>
        </Box>
      </Box>

      <Box marginTop={1}>
        <Text dimColor>
          CLI Commands: raisindb package create {'<folder>'} | raisindb package upload {'<file>'}
        </Text>
      </Box>

      <Box marginTop={1}>
        <Text dimColor>
          Tips: ↑/↓ for history | Tab for auto-complete | Double-tab to list all
        </Text>
      </Box>
    </Box>
  );
};

export default HelpScreen;
