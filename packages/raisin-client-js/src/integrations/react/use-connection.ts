import type { ReactLikeWithContext, ReactContext, RaisinContextValue, UseConnectionReturn } from './types';
import { ConnectionState } from '../../connection';

export function createUseConnection(
  react: ReactLikeWithContext,
  context: ReactContext<RaisinContextValue | null>,
): () => UseConnectionReturn {
  return function useConnection(): UseConnectionReturn {
    const { useState, useEffect, useCallback, useContext } = react;

    const ctx = useContext(context);
    if (!ctx) {
      throw new Error('useConnection must be used within a <RaisinProvider>');
    }
    const { client } = ctx;

    const [state, setState] = useState<ConnectionState>(client.getConnectionState());
    const [isReady, setIsReady] = useState(client.isReady());

    useEffect(() => {
      const unsubConn = client.onConnectionStateChange((newState) => {
        setState(newState);
      });

      const unsubReady = client.onReadyStateChange((ready) => {
        setIsReady(ready);
      });

      return () => {
        unsubConn();
        unsubReady();
      };
    }, [client]);

    const connect = useCallback(async () => {
      await client.connect();
    }, [client]);

    const disconnect = useCallback(() => {
      client.disconnect();
    }, [client]);

    return {
      state,
      isConnected: state === ConnectionState.Connected,
      isReady,
      connect,
      disconnect,
    };
  };
}
