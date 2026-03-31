import React from 'react';
import { Box, Text } from 'ink';
import InkSpinner from 'ink-spinner';

interface SpinnerProps {
  text?: string;
}

const Spinner: React.FC<SpinnerProps> = ({ text = 'Loading...' }) => {
  return (
    <Box>
      <Text color="cyan">
        <InkSpinner type="dots" />
      </Text>
      <Text> {text}</Text>
    </Box>
  );
};

export default Spinner;
