import type { ReactLikeWithContext, ReactContext, RaisinContextValue, UseAuthReturn } from './types';
import type { IdentityUser } from '../../auth';

export function createUseAuth(
  react: ReactLikeWithContext,
  context: ReactContext<RaisinContextValue | null>,
): () => UseAuthReturn {
  return function useAuth(): UseAuthReturn {
    const { useState, useEffect, useRef, useCallback, useContext } = react;

    const ctx = useContext(context);
    if (!ctx) {
      throw new Error('useAuth must be used within a <RaisinProvider>');
    }
    const { client } = ctx;

    const [user, setUser] = useState<IdentityUser | null>(client.getStoredUser());
    const [isLoading, setIsLoading] = useState(false);
    const mountedRef = useRef(true);

    useEffect(() => {
      mountedRef.current = true;

      const unsubAuth = client.onAuthStateChange(({ event, session }) => {
        if (!mountedRef.current) return;
        if (event === 'SIGNED_IN' || event === 'TOKEN_REFRESHED' || event === 'USER_UPDATED') {
          setUser(session.user);
        } else if (event === 'SIGNED_OUT' || event === 'SESSION_EXPIRED') {
          setUser(null);
        }
      });

      const unsubUser = client.onUserChange(({ node }) => {
        if (!mountedRef.current) return;
        // Update user with the latest node properties
        setUser((prev) => {
          if (!prev) return prev;
          return {
            ...prev,
            home: node.path,
            displayName: (node.properties.display_name as string) ?? prev.displayName,
            avatarUrl: (node.properties.avatar_url as string) ?? prev.avatarUrl,
          };
        });
      });

      return () => {
        mountedRef.current = false;
        unsubAuth();
        unsubUser();
      };
    }, [client]);

    const login = useCallback(async (email: string, password: string, repository: string) => {
      setIsLoading(true);
      try {
        const result = await client.loginWithEmail(email, password, repository);
        return result;
      } finally {
        if (mountedRef.current) setIsLoading(false);
      }
    }, [client]);

    const register = useCallback(async (email: string, password: string, repository: string, displayName?: string) => {
      setIsLoading(true);
      try {
        const result = await client.registerWithEmail(email, password, repository, displayName);
        return result;
      } finally {
        if (mountedRef.current) setIsLoading(false);
      }
    }, [client]);

    const logout = useCallback(async (options?: { disconnect?: boolean; reconnect?: boolean }) => {
      setIsLoading(true);
      try {
        await client.logout(options);
      } finally {
        if (mountedRef.current) setIsLoading(false);
      }
    }, [client]);

    const initSession = useCallback(async (repository: string) => {
      setIsLoading(true);
      try {
        const result = await client.initSession(repository);
        return result;
      } finally {
        if (mountedRef.current) setIsLoading(false);
      }
    }, [client]);

    return {
      user,
      isAuthenticated: user !== null,
      isLoading,
      login,
      register,
      logout,
      initSession,
    };
  };
}
