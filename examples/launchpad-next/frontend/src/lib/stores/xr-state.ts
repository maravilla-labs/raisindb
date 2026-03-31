import { writable, derived, type Readable } from 'svelte/store';

export type XRSessionState = 'idle' | 'starting' | 'active' | 'ending' | 'error';

interface XRStoreState {
  sessionState: XRSessionState;
  session: XRSession | null;
  error: string | null;
  handTrackingSupported: boolean;
  leftHandConnected: boolean;
  rightHandConnected: boolean;
}

function createXRStore() {
  const { subscribe, set, update } = writable<XRStoreState>({
    sessionState: 'idle',
    session: null,
    error: null,
    handTrackingSupported: false,
    leftHandConnected: false,
    rightHandConnected: false
  });

  return {
    subscribe,

    setSession(session: XRSession | null) {
      update(state => ({
        ...state,
        session,
        sessionState: session ? 'active' : 'idle',
        error: null
      }));
    },

    setSessionState(sessionState: XRSessionState) {
      update(state => ({ ...state, sessionState }));
    },

    setError(error: string | null) {
      update(state => ({
        ...state,
        error,
        sessionState: error ? 'error' : state.sessionState
      }));
    },

    setHandTrackingSupported(supported: boolean) {
      update(state => ({ ...state, handTrackingSupported: supported }));
    },

    setHandConnected(hand: 'left' | 'right', connected: boolean) {
      update(state => ({
        ...state,
        [hand === 'left' ? 'leftHandConnected' : 'rightHandConnected']: connected
      }));
    },

    reset() {
      set({
        sessionState: 'idle',
        session: null,
        error: null,
        handTrackingSupported: false,
        leftHandConnected: false,
        rightHandConnected: false
      });
    }
  };
}

export const xrStore = createXRStore();

// Derived stores for convenience
export const isXRActive: Readable<boolean> = derived(
  xrStore,
  $xr => $xr.sessionState === 'active'
);

export const xrSession: Readable<XRSession | null> = derived(
  xrStore,
  $xr => $xr.session
);

export const xrError: Readable<string | null> = derived(
  xrStore,
  $xr => $xr.error
);
