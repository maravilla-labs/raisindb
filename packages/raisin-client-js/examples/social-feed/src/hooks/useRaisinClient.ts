import { useEffect, useState } from 'react';
import { RaisinClient } from '@raisindb/client';
import { getClient } from '../lib/raisin';

export function useRaisinClient() {
  const [client, setClient] = useState<RaisinClient | null>(null);
  const [isConnected, setIsConnected] = useState(false);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    let mounted = true;
    let cleanup: (() => void) | undefined;

    async function initialize() {
      try {
        const raisinClient = await getClient();
        if (mounted) {
          setClient(raisinClient);
          setIsConnected(raisinClient.isConnected());

          // Listen for connection state changes
          const handleStateChange = () => {
            if (mounted) {
              setIsConnected(raisinClient.isConnected());
            }
          };

          // Subscribe to state changes
          (raisinClient as any).connection.on('stateChange', handleStateChange);

          // Store cleanup function
          cleanup = () => {
            (raisinClient as any).connection.off('stateChange', handleStateChange);
          };
        }
      } catch (err) {
        if (mounted) {
          setError(err as Error);
        }
      }
    }

    initialize();

    return () => {
      mounted = false;
      if (cleanup) cleanup();
    };
  }, []);

  return { client, isConnected, error };
}
