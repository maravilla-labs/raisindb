/**
 * Voice Activation Store - Manages "Hey Computer" voice activation
 *
 * Uses Web Speech API for fast, real-time keyword detection.
 * No model downloads required - uses browser's native speech recognition.
 *
 * Flow:
 * 1. 'listening' - Waiting for "Hey Computer" wake word
 * 2. 'activated' - Wake word detected, plays chime, opens AI chat
 * 3. 'capturing' - Continues listening for the user's command
 * 4. When command is complete, sends it to AI chat
 * 5. Returns to 'listening'
 */
import { writable, derived, get } from 'svelte/store';
import { browser } from '$app/environment';
import { aiChatStore } from './ai-chat';
import { currentPageContext } from './navigation';

// ============================================================================
// Types
// ============================================================================

export type VoiceState = 'idle' | 'listening' | 'activated' | 'capturing' | 'error';

interface VoiceActivationState {
  state: VoiceState;
  transcript: string;
  command: string; // The command being captured after wake word
  lastKeyword: string | null;
  error: string | null;
  isSupported: boolean;
}

// ============================================================================
// Constants
// ============================================================================

// Keyword patterns to detect (fuzzy matching for speech recognition quirks)
const KEYWORD_PATTERNS = [
  /hey[,.]?\s*computer/i,
  /hay[,.]?\s*computer/i,
  /a[,.]?\s*computer/i,
];

// How long to wait for command after wake word (ms)
const COMMAND_TIMEOUT = 5000;

// ============================================================================
// Store
// ============================================================================

const initialState: VoiceActivationState = {
  state: 'idle',
  transcript: '',
  command: '',
  lastKeyword: null,
  error: null,
  isSupported: false,
};

const store = writable<VoiceActivationState>(initialState);

// Speech recognition instance
let recognition: SpeechRecognition | null = null;
let commandTimeout: ReturnType<typeof setTimeout> | null = null;

// ============================================================================
// Helpers
// ============================================================================

function detectKeyword(text: string): boolean {
  const normalized = text.toLowerCase().trim();
  return KEYWORD_PATTERNS.some((pattern) => pattern.test(normalized));
}

/**
 * Extract the command part after the wake word
 */
function extractCommand(text: string): string {
  const normalized = text.toLowerCase();

  // Find where the wake word ends
  for (const pattern of KEYWORD_PATTERNS) {
    const match = normalized.match(pattern);
    if (match) {
      const endIndex = match.index! + match[0].length;
      const command = text.slice(endIndex).trim();
      return command;
    }
  }

  return text.trim();
}

function updateState(partial: Partial<VoiceActivationState>) {
  store.update((s) => ({ ...s, ...partial }));
}

/**
 * Open AI chat with current page context
 * Uses the full node path from navigation store for AI context
 */
function openAIChatWithContext() {
  if (!browser) return;

  const pageContext = get(currentPageContext);

  if (pageContext) {
    // Use the full node path from the page context
    const isBoard = pageContext.archetype?.includes('KanbanBoard') ||
                    pageContext.nodePath.includes('/boards/');

    aiChatStore.setContext({
      type: isBoard ? 'board' : 'page',
      path: pageContext.nodePath, // Full node path like /launchpad/boards/my-board
      boardSlug: isBoard ? pageContext.nodePath.split('/').pop() : undefined,
      title: pageContext.title,
      nodeType: pageContext.nodeType,
      archetype: pageContext.archetype,
    });
  } else {
    // Fallback to URL path if no page context
    const urlPath = window.location.pathname;
    aiChatStore.setContext({
      type: 'page',
      path: urlPath,
    });
  }

  aiChatStore.open();
}

/**
 * Send the captured command to AI chat
 */
async function sendCommandToAI(command: string) {
  if (!command.trim()) {
    console.log('[Voice] No command to send');
    return;
  }

  console.log('[Voice] Sending command to AI:', command);

  // Get context to include in message
  const context = aiChatStore.getContext();
  let messageContent = command;

  if (context) {
    if (context.type === 'board') {
      messageContent = `[Context: I'm on board "${context.boardSlug}" at ${context.path}]\n\n${command}`;
    } else {
      messageContent = `[Context: I'm viewing ${context.path}]\n\n${command}`;
    }
  }

  // Create a new conversation if none exists, then send message
  const state = get(aiChatStore);
  if (!state.activeConversationId) {
    await aiChatStore.createConversation();
  }

  await aiChatStore.sendMessage(messageContent);
}

/**
 * Play activation chime
 */
async function playActivationSound() {
  if (!browser) return;

  try {
    const ctx = new AudioContext();
    const now = ctx.currentTime;

    // First tone (C5)
    const osc1 = ctx.createOscillator();
    const gain1 = ctx.createGain();
    osc1.type = 'sine';
    osc1.frequency.value = 523.25;
    gain1.gain.setValueAtTime(0.3, now);
    gain1.gain.exponentialRampToValueAtTime(0.01, now + 0.15);
    osc1.connect(gain1);
    gain1.connect(ctx.destination);
    osc1.start(now);
    osc1.stop(now + 0.15);

    // Second tone (E5)
    const osc2 = ctx.createOscillator();
    const gain2 = ctx.createGain();
    osc2.type = 'sine';
    osc2.frequency.value = 659.25;
    gain2.gain.setValueAtTime(0.3, now + 0.08);
    gain2.gain.exponentialRampToValueAtTime(0.01, now + 0.25);
    osc2.connect(gain2);
    gain2.connect(ctx.destination);
    osc2.start(now + 0.08);
    osc2.stop(now + 0.25);

    // Third tone (G5)
    const osc3 = ctx.createOscillator();
    const gain3 = ctx.createGain();
    osc3.type = 'sine';
    osc3.frequency.value = 783.99;
    gain3.gain.setValueAtTime(0.25, now + 0.15);
    gain3.gain.exponentialRampToValueAtTime(0.01, now + 0.35);
    osc3.connect(gain3);
    gain3.connect(ctx.destination);
    osc3.start(now + 0.15);
    osc3.stop(now + 0.35);

    setTimeout(() => ctx.close(), 500);
  } catch (e) {
    console.warn('Could not play activation sound:', e);
  }
}

/**
 * Clear command timeout
 */
function clearCommandTimeout() {
  if (commandTimeout) {
    clearTimeout(commandTimeout);
    commandTimeout = null;
  }
}

// ============================================================================
// Public API
// ============================================================================

function createVoiceActivationStore() {
  const { subscribe } = store;

  return {
    subscribe,

    checkSupport(): boolean {
      if (!browser) return false;

      const SpeechRecognition =
        (window as any).SpeechRecognition || (window as any).webkitSpeechRecognition;

      const supported = !!SpeechRecognition;
      updateState({ isSupported: supported });
      return supported;
    },

    async startListening(): Promise<boolean> {
      if (!browser) return false;

      const currentState = get(store);
      if (currentState.state === 'listening' || currentState.state === 'capturing') return true;

      const SpeechRecognition =
        (window as any).SpeechRecognition || (window as any).webkitSpeechRecognition;

      if (!SpeechRecognition) {
        updateState({
          state: 'error',
          error: 'Speech recognition not supported in this browser',
        });
        return false;
      }

      try {
        recognition = new SpeechRecognition();
        recognition.continuous = true;
        recognition.interimResults = true;
        recognition.lang = 'en-US';

        recognition.onresult = (event: SpeechRecognitionEvent) => {
          const lastResult = event.results[event.results.length - 1];
          const transcript = lastResult[0].transcript;
          const isFinal = lastResult.isFinal;

          console.log('[Voice] Heard:', transcript, 'Final:', isFinal);

          const currentState = get(store);

          if (currentState.state === 'listening') {
            // Looking for wake word
            updateState({ transcript });

            if (detectKeyword(transcript)) {
              console.log('[Voice] WAKE WORD DETECTED!');

              // Extract any command that came with the wake word
              const immediateCommand = extractCommand(transcript);

              updateState({
                state: 'activated',
                lastKeyword: transcript,
                command: immediateCommand,
              });

              playActivationSound();
              openAIChatWithContext();

              // Transition to capturing state after a brief moment
              setTimeout(() => {
                const s = get(store);
                if (s.state === 'activated') {
                  updateState({ state: 'capturing', transcript: '' });

                  // Set timeout for command capture
                  commandTimeout = setTimeout(() => {
                    const s = get(store);
                    if (s.state === 'capturing') {
                      // Send whatever we have
                      if (s.command.trim()) {
                        sendCommandToAI(s.command);
                      }
                      updateState({ state: 'listening', command: '', transcript: '' });
                    }
                  }, COMMAND_TIMEOUT);
                }
              }, 500);
            }
          } else if (currentState.state === 'capturing') {
            // Capturing the command after wake word
            updateState({ command: transcript, transcript });

            if (isFinal && transcript.trim()) {
              console.log('[Voice] Command captured:', transcript);
              clearCommandTimeout();

              // Send to AI
              sendCommandToAI(transcript);

              // Reset to listening
              updateState({ state: 'listening', command: '', transcript: '' });
            }
          } else if (currentState.state === 'activated') {
            // Still in activated state, accumulate command
            const command = extractCommand(transcript);
            updateState({ command });
          }
        };

        recognition.onerror = (event: SpeechRecognitionErrorEvent) => {
          console.error('[Voice] Recognition error:', event.error);

          if (event.error === 'no-speech' || event.error === 'aborted') {
            return;
          }

          updateState({
            state: 'error',
            error: event.error,
          });
        };

        recognition.onend = () => {
          console.log('[Voice] Recognition ended');

          const s = get(store);
          if ((s.state === 'listening' || s.state === 'capturing') && recognition) {
            console.log('[Voice] Auto-restarting...');
            try {
              recognition.start();
            } catch (e) {
              console.warn('[Voice] Could not restart:', e);
            }
          }
        };

        recognition.start();
        console.log('[Voice] Started listening');

        updateState({
          state: 'listening',
          error: null,
          transcript: '',
          command: '',
        });

        return true;
      } catch (error) {
        console.error('Failed to start listening:', error);
        updateState({
          state: 'error',
          error: error instanceof Error ? error.message : 'Failed to access microphone',
        });
        return false;
      }
    },

    stopListening(): void {
      clearCommandTimeout();

      if (recognition) {
        recognition.stop();
        recognition = null;
      }

      updateState({
        state: 'idle',
        transcript: '',
        command: '',
      });
    },

    async toggle(): Promise<boolean> {
      const currentState = get(store);

      if (currentState.state === 'listening' ||
          currentState.state === 'activated' ||
          currentState.state === 'capturing') {
        this.stopListening();
        return false;
      } else {
        return this.startListening();
      }
    },

    reset(): void {
      clearCommandTimeout();
      updateState({
        lastKeyword: null,
        transcript: '',
        command: '',
      });

      const currentState = get(store);
      if (currentState.state === 'activated' || currentState.state === 'capturing') {
        updateState({ state: 'listening' });
      }
    },

    dispose(): void {
      this.stopListening();
      updateState(initialState);
    },
  };
}

export const voiceActivationStore = createVoiceActivationStore();

// ============================================================================
// Derived Stores
// ============================================================================

export const voiceState = derived(store, ($s) => $s.state);
export const voiceTranscript = derived(store, ($s) => $s.transcript);
export const voiceCommand = derived(store, ($s) => $s.command);
export const voiceLastKeyword = derived(store, ($s) => $s.lastKeyword);
export const voiceError = derived(store, ($s) => $s.error);
export const isVoiceSupported = derived(store, ($s) => $s.isSupported);

export const isVoiceListening = derived(
  store,
  ($s) => $s.state === 'listening' || $s.state === 'activated' || $s.state === 'capturing'
);

export const isVoiceActivated = derived(
  store,
  ($s) => $s.state === 'activated' || $s.state === 'capturing'
);

export const isVoiceCapturing = derived(store, ($s) => $s.state === 'capturing');

// Deprecated - kept for backwards compatibility
export const isVoiceLoading = derived(store, () => false);
export const voiceLoadProgress = derived(store, () => 100);
export const voiceLoadStatus = derived(store, () => 'Ready');
