import React from 'react';
import { Box, Text } from 'ink';
import Gradient from 'ink-gradient';
import BigText from 'ink-big-text';

// Brand colors from admin-console: brown/amber/orange theme
const BRAND_COLORS = ['#B8754E', '#D97706', '#EA580C', '#f97316'];

const Banner: React.FC = () => {
  return (
    <Box flexDirection="column" paddingX={2} paddingY={1}>
      <Gradient colors={BRAND_COLORS}>
        <BigText text="RaisinDB" font="block" />
      </Gradient>
      <Box marginTop={1}>
        <Text color="#B8754E">Interactive CLI v0.1.0</Text>
        <Text dimColor>  •  Type /help for commands</Text>
      </Box>
    </Box>
  );
};

export default Banner;
