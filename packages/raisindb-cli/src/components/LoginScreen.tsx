import React, { useState, useEffect } from 'react';
import { Box, Text, useInput } from 'ink';
import Spinner from './Spinner.js';
import { login } from '../auth.js';

interface LoginScreenProps {
  serverUrl: string;
  onSuccess: (token: string) => void;
  onCancel: () => void;
  onError: (error: string) => void;
}

const LOGIN_TIMEOUT_SECONDS = 120; // 2 minutes

const LoginScreen: React.FC<LoginScreenProps> = ({ serverUrl, onSuccess, onCancel, onError }) => {
  const [status, setStatus] = useState<'opening' | 'waiting' | 'success' | 'error'>('opening');
  const [secondsRemaining, setSecondsRemaining] = useState(LOGIN_TIMEOUT_SECONDS);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // Handle ESC key to cancel
  useInput((input, key) => {
    if (key.escape) {
      onCancel();
    }
  });

  // Countdown timer
  useEffect(() => {
    if (status !== 'waiting') return;

    const interval = setInterval(() => {
      setSecondsRemaining(prev => {
        if (prev <= 1) {
          clearInterval(interval);
          setStatus('error');
          setErrorMessage('Authentication timeout');
          onError('Authentication timeout - no response received');
          return 0;
        }
        return prev - 1;
      });
    }, 1000);

    return () => clearInterval(interval);
  }, [status, onError]);

  // Start login process
  useEffect(() => {
    let cancelled = false;

    const doLogin = async () => {
      try {
        setStatus('waiting');
        const token = await login(serverUrl);
        if (!cancelled) {
          setStatus('success');
          onSuccess(token);
        }
      } catch (error) {
        if (!cancelled) {
          const msg = error instanceof Error ? error.message : String(error);
          // Don't show error if it was cancelled
          if (!msg.includes('cancelled')) {
            setStatus('error');
            setErrorMessage(msg);
            onError(msg);
          }
        }
      }
    };

    doLogin();

    return () => {
      cancelled = true;
    };
  }, [serverUrl, onSuccess, onError]);

  // Format time remaining
  const formatTime = (seconds: number): string => {
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${mins}:${secs.toString().padStart(2, '0')}`;
  };

  if (status === 'error') {
    return (
      <Box flexDirection="column">
        <Box>
          <Text color="red">✖ </Text>
          <Text color="red">{errorMessage || 'Authentication failed'}</Text>
        </Box>
      </Box>
    );
  }

  if (status === 'success') {
    return (
      <Box>
        <Text color="green">✓ </Text>
        <Text color="green">Login successful!</Text>
      </Box>
    );
  }

  return (
    <Box flexDirection="column">
      <Box>
        <Spinner text="Waiting for browser authentication..." />
      </Box>
      <Box marginTop={1}>
        <Text dimColor>
          Timeout in {formatTime(secondsRemaining)} • Press </Text>
        <Text color="yellow">ESC</Text>
        <Text dimColor> to cancel</Text>
      </Box>
    </Box>
  );
};

export default LoginScreen;
