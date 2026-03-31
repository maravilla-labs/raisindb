/**
 * Whisper WASM wrapper using transformers.js via Web Worker
 * Provides speech-to-text transcription for voice activation
 *
 * Uses a Web Worker to isolate transformers.js from SvelteKit's bundling
 */

import { browser } from '$app/environment';

// Singleton worker instance
let worker: Worker | null = null;
let isLoading = false;
let isReady = false;
let loadError: Error | null = null;
let messageId = 0;
const pendingRequests = new Map<number, { resolve: Function; reject: Function }>();
let progressCallback: ((progress: LoadProgress) => void) | null = null;

export interface TranscriptionResult {
  text: string;
  chunks?: Array<{
    text: string;
    timestamp: [number, number];
  }>;
}

export interface LoadProgress {
  status: 'initiate' | 'download' | 'progress' | 'done' | 'ready';
  name?: string;
  file?: string;
  progress?: number;
  loaded?: number;
  total?: number;
}

/**
 * Create and initialize the worker
 */
function createWorker(): Worker {
  const workerCode = `
    import { pipeline, env } from 'https://cdn.jsdelivr.net/npm/@xenova/transformers@2.17.2';

    // Configure environment
    env.allowLocalModels = false;
    env.useBrowserCache = true;

    const MODEL_ID = 'Xenova/whisper-base.en';
    let transcriber = null;

    self.onmessage = async (e) => {
      const { type, data, id } = e.data;
      console.log('[Whisper Worker] Received message:', type, id);

      try {
        switch (type) {
          case 'init':
            console.log('[Whisper Worker] Initializing model...');
            if (!transcriber) {
              transcriber = await pipeline('automatic-speech-recognition', MODEL_ID, {
                progress_callback: (progress) => {
                  self.postMessage({
                    type: 'progress',
                    data: {
                      status: progress.status,
                      name: progress.name,
                      file: progress.file,
                      progress: progress.progress,
                      loaded: progress.loaded,
                      total: progress.total,
                    },
                  });
                },
              });
            }
            console.log('[Whisper Worker] Model initialized');
            self.postMessage({ type: 'init-complete', id });
            break;

          case 'transcribe':
            if (!transcriber) {
              throw new Error('Model not initialized');
            }
            // Convert back to Float32Array if needed (structured clone converts it)
            const audioData = data.audio instanceof Float32Array
              ? data.audio
              : new Float32Array(data.audio);
            console.log('[Whisper Worker] Transcribing', audioData.length, 'samples');
            const result = await transcriber(audioData, {
              chunk_length_s: 30,
              stride_length_s: 5,
              return_timestamps: false,
            });
            console.log('[Whisper Worker] Result:', result.text);
            self.postMessage({
              type: 'transcribe-result',
              id,
              data: { text: result.text?.trim() || '', chunks: result.chunks }
            });
            break;

          default:
            self.postMessage({ type: 'error', id, error: 'Unknown message type: ' + type });
        }
      } catch (error) {
        console.error('[Whisper Worker] Error:', error);
        self.postMessage({
          type: 'error',
          id,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    };
  `;

  const blob = new Blob([workerCode], { type: 'application/javascript' });
  const workerUrl = URL.createObjectURL(blob);
  const w = new Worker(workerUrl, { type: 'module' });

  w.onmessage = (e: MessageEvent) => {
    const { type, id, data, error } = e.data;
    console.log('[Whisper] Worker message received:', type, id);

    if (type === 'progress' && progressCallback) {
      progressCallback(data);
      return;
    }

    const pending = pendingRequests.get(id);
    if (!pending) {
      console.log('[Whisper] No pending request for id:', id);
      return;
    }

    pendingRequests.delete(id);

    if (type === 'error') {
      console.error('[Whisper] Worker error:', error);
      pending.reject(new Error(error));
    } else {
      console.log('[Whisper] Resolving with data:', data);
      pending.resolve(data);
    }
  };

  w.onerror = (e) => {
    console.error('Worker error:', e);
    loadError = new Error(e.message || 'Worker error');
  };

  return w;
}

/**
 * Send a message to the worker and wait for response
 */
function sendMessage(type: string, data?: any): Promise<any> {
  return new Promise((resolve, reject) => {
    if (!worker) {
      reject(new Error('Worker not initialized'));
      return;
    }

    const id = ++messageId;
    pendingRequests.set(id, { resolve, reject });
    worker.postMessage({ type, data, id });
  });
}

/**
 * Initialize the Whisper model
 * Downloads ~40MB model on first use (cached after)
 */
export async function initWhisper(
  onProgress?: (progress: LoadProgress) => void
): Promise<boolean> {
  if (!browser) {
    console.warn('Whisper can only be initialized in the browser');
    return false;
  }

  if (isReady) {
    return true;
  }

  if (isLoading) {
    // Wait for existing load to complete
    while (isLoading) {
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return isReady;
  }

  isLoading = true;
  loadError = null;
  progressCallback = onProgress || null;

  try {
    if (!worker) {
      worker = createWorker();
    }

    await sendMessage('init');

    if (onProgress) {
      onProgress({ status: 'ready' });
    }

    isReady = true;
    return true;
  } catch (error) {
    loadError = error as Error;
    console.error('Failed to initialize Whisper:', error);
    return false;
  } finally {
    isLoading = false;
  }
}

/**
 * Check if the model is loaded and ready
 */
export function isWhisperReady(): boolean {
  return isReady;
}

/**
 * Check if the model is currently loading
 */
export function isWhisperLoading(): boolean {
  return isLoading;
}

/**
 * Get any load error that occurred
 */
export function getWhisperError(): Error | null {
  return loadError;
}

/**
 * Transcribe audio data to text
 * @param audioData - Float32Array of audio samples at 16kHz sample rate
 * @returns Transcription result with text
 */
export async function transcribe(
  audioData: Float32Array
): Promise<TranscriptionResult> {
  if (!isReady || !worker) {
    throw new Error('Whisper not initialized. Call initWhisper() first.');
  }

  console.log('[Whisper] Transcribing audio:', audioData.length, 'samples');

  try {
    // Transfer the audio data to the worker (not just copy)
    const result = await sendMessage('transcribe', { audio: audioData });
    console.log('[Whisper] Transcription complete:', result);
    return result;
  } catch (error) {
    console.error('[Whisper] Transcription error:', error);
    throw error;
  }
}

/**
 * Transcribe from a Blob (e.g., from MediaRecorder)
 * Converts the blob to the required format for Whisper
 */
export async function transcribeBlob(blob: Blob): Promise<TranscriptionResult> {
  const arrayBuffer = await blob.arrayBuffer();
  const audioContext = new AudioContext({ sampleRate: 16000 });
  const audioBuffer = await audioContext.decodeAudioData(arrayBuffer);

  // Get mono channel data
  const channelData = audioBuffer.getChannelData(0);

  // Resample to 16kHz if needed
  const targetSampleRate = 16000;
  let audioData: Float32Array;

  if (audioBuffer.sampleRate !== targetSampleRate) {
    const ratio = audioBuffer.sampleRate / targetSampleRate;
    const newLength = Math.round(channelData.length / ratio);
    audioData = new Float32Array(newLength);

    for (let i = 0; i < newLength; i++) {
      const index = Math.round(i * ratio);
      audioData[i] = channelData[Math.min(index, channelData.length - 1)];
    }
  } else {
    audioData = channelData;
  }

  await audioContext.close();
  return transcribe(audioData);
}

/**
 * Clean up resources
 */
export function disposeWhisper(): void {
  if (worker) {
    worker.terminate();
    worker = null;
  }
  isReady = false;
  loadError = null;
  progressCallback = null;
  pendingRequests.clear();
}
