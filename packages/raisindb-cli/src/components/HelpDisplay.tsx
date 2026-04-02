import React from 'react';
import { Box, Text } from 'ink';
import Gradient from 'ink-gradient';

const BRAND_COLORS = ['#B8754E', '#D97706', '#EA580C', '#f97316'];

const commands = [
  { group: 'Server', items: [
    { cmd: 'server start', desc: 'Start the development server' },
    { cmd: 'server stop', desc: 'Stop a running server' },
    { cmd: 'server status', desc: 'Check server health' },
    { cmd: 'server logs', desc: 'View server logs' },
    { cmd: 'server install', desc: 'Download the server binary' },
    { cmd: 'server update', desc: 'Update to latest version' },
  ]},
  { group: 'Packages', items: [
    { cmd: 'package create <dir>', desc: 'Create a .rap package' },
    { cmd: 'package install <name>', desc: 'Install a package' },
    { cmd: 'package sync', desc: 'Sync package with server' },
    { cmd: 'package clone', desc: 'Clone package from server' },
  ]},
  { group: 'Interactive', items: [
    { cmd: 'shell', desc: 'Start interactive SQL shell' },
  ]},
];

export function HelpDisplay({ version }: { version: string }) {
  return (
    <Box flexDirection="column" paddingX={2} paddingY={1}>
      <Box marginBottom={1}>
        <Gradient colors={BRAND_COLORS}>
          <Text bold> RaisinDB </Text>
        </Gradient>
        <Text dimColor> CLI v{version}</Text>
      </Box>

      {commands.map((group) => (
        <Box key={group.group} flexDirection="column" marginBottom={1}>
          <Text bold color="#D97706">{group.group}</Text>
          {group.items.map((item) => (
            <Box key={item.cmd} paddingLeft={2}>
              <Box width={28}>
                <Text color="white">{item.cmd}</Text>
              </Box>
              <Text dimColor>{item.desc}</Text>
            </Box>
          ))}
        </Box>
      ))}

      <Box marginTop={1} flexDirection="column">
        <Text dimColor>Run </Text>
        <Box>
          <Text dimColor>  raisindb </Text>
          <Text color="white">{'<command>'}</Text>
          <Text dimColor> --help for details</Text>
        </Box>
        <Box>
          <Text dimColor>  Docs: </Text>
          <Text color="cyan">https://raisindb.com/docs</Text>
        </Box>
      </Box>
    </Box>
  );
}
