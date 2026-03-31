import { useState, useEffect } from 'react';
import { useRaisinClient } from './useRaisinClient';

export function useAuth() {
  const { client, isConnected } = useRaisinClient();
  const [isAuthenticated, setIsAuthenticated] = useState(false);
  const [currentUser, setCurrentUser] = useState<any>(null);

  useEffect(() => {
    let mounted = true;

    if (client && isConnected) {
      // Check authentication status immediately
      const checkAuth = () => {
        if (!mounted) return;
        const authStatus = client.isAuthenticated();
        setIsAuthenticated(authStatus);

        if (authStatus) {
          // In a real app, we'd fetch the current user profile here
          setCurrentUser({
            username: 'example',
            displayName: 'Example User',
          });
        }
      };

      checkAuth();

      // Listen for authenticated event
      const handleAuthenticated = () => {
        if (!mounted) return;
        console.log('🎉 Authentication event received');
        setIsAuthenticated(true);
        setCurrentUser({
          username: 'example',
          displayName: 'Example User',
        });
      };

      (client as any).on('authenticated', handleAuthenticated);

      return () => {
        mounted = false;
        (client as any).off('authenticated', handleAuthenticated);
      };
    } else {
      // Reset authentication state when not connected
      setIsAuthenticated(false);
      setCurrentUser(null);
    }
  }, [client, isConnected]);

  return {
    isAuthenticated,
    currentUser,
    client,
    isConnected,
  };
}
