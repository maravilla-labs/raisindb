import React from 'react';
import { Box, Text } from 'ink';

export interface StatusMessageProps {
  message: string;
}

export const SuccessMessage: React.FC<StatusMessageProps> = ({ message }) => {
  return (
    <Box>
      <Text color="green">✔ </Text>
      <Text>{message}</Text>
    </Box>
  );
};

export const ErrorMessage: React.FC<StatusMessageProps> = ({ message }) => {
  return (
    <Box>
      <Text color="red">✖ </Text>
      <Text color="red">{message}</Text>
    </Box>
  );
};

export const WarningMessage: React.FC<StatusMessageProps> = ({ message }) => {
  return (
    <Box>
      <Text color="yellow">⚠ </Text>
      <Text color="yellow">{message}</Text>
    </Box>
  );
};

export const InfoMessage: React.FC<StatusMessageProps> = ({ message }) => {
  return (
    <Box>
      <Text color="blue">ℹ </Text>
      <Text>{message}</Text>
    </Box>
  );
};
