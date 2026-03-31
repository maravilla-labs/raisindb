<script lang="ts">
  import { Mic, MicOff, Sparkles, AlertCircle, X } from 'lucide-svelte';
  import { onMount } from 'svelte';
  import { page } from '$app/stores';
  import {
    voiceActivationStore,
    voiceState,
    voiceTranscript,
    voiceCommand,
    voiceLastKeyword,
    voiceError,
    isVoiceSupported,
    isVoiceListening,
    isVoiceActivated,
    isVoiceCapturing,
  } from '$lib/stores/voice-activation';
  import { user } from '$lib/stores/auth';

  let showPanel = $state(false);
  let panelRef: HTMLDivElement | null = $state(null);

  // Track if on a board page for context
  const boardPath = $derived(() => {
    const pathname = $page.url.pathname;
    // Check if we're on a board page (will be useful for future AI integration)
    return pathname;
  });

  function handleClickOutside(e: MouseEvent) {
    if (panelRef && !panelRef.contains(e.target as Node)) {
      showPanel = false;
    }
  }

  $effect(() => {
    if (showPanel) {
      document.addEventListener('click', handleClickOutside);
      return () => document.removeEventListener('click', handleClickOutside);
    }
  });

  onMount(() => {
    voiceActivationStore.checkSupport();
    return () => {
      voiceActivationStore.dispose();
    };
  });

  async function handleToggle() {
    if ($voiceState === 'idle' || $voiceState === 'error') {
      showPanel = true;
      await voiceActivationStore.startListening();
    } else if ($isVoiceListening) {
      voiceActivationStore.stopListening();
      showPanel = false;
    }
  }

  function handleClose() {
    voiceActivationStore.stopListening();
    showPanel = false;
  }

  function handleRetry() {
    voiceActivationStore.startListening();
  }

  // Format transcript for display (last 100 chars)
  const displayTranscript = $derived(
    $voiceTranscript.length > 100
      ? '...' + $voiceTranscript.slice(-100)
      : $voiceTranscript
  );
</script>

{#if $user && $isVoiceSupported}
  <div class="voice-activation" bind:this={panelRef}>
    <button
      class="voice-button"
      class:listening={$isVoiceListening}
      class:activated={$isVoiceActivated}
      class:error={$voiceState === 'error'}
      onclick={handleToggle}
      title={$isVoiceListening ? 'Stop listening' : 'Start voice activation'}
      aria-label="Voice activation"
    >
      {#if $isVoiceActivated}
        <Sparkles size={20} />
      {:else if $voiceState === 'error'}
        <AlertCircle size={20} />
      {:else if $isVoiceListening}
        <Mic size={20} />
      {:else}
        <MicOff size={20} />
      {/if}

      {#if $isVoiceListening && !$isVoiceActivated}
        <span class="pulse-ring"></span>
      {/if}
    </button>

    {#if showPanel && ($isVoiceListening || $voiceState === 'error' || $isVoiceActivated)}
      <div class="voice-panel">
        <div class="panel-header">
          <span class="panel-title">
            {#if $isVoiceCapturing}
              Speak your command...
            {:else if $voiceState === 'activated'}
              Hey Computer!
            {:else if $voiceState === 'error'}
              Error
            {:else}
              Listening...
            {/if}
          </span>
          <button class="close-btn" onclick={handleClose} title="Close">
            <X size={16} />
          </button>
        </div>

        <div class="panel-content">
          {#if $voiceState === 'error'}
            <div class="error-state">
              <AlertCircle size={24} />
              <p>{$voiceError || 'An error occurred'}</p>
              <button class="retry-btn" onclick={handleRetry}>Try Again</button>
            </div>
          {:else if $isVoiceCapturing}
            <div class="capturing-state">
              <div class="waveform capturing">
                <span class="wave-bar"></span>
                <span class="wave-bar"></span>
                <span class="wave-bar"></span>
                <span class="wave-bar"></span>
                <span class="wave-bar"></span>
              </div>
              <p class="capturing-hint">Listening for your command...</p>
              {#if $voiceCommand}
                <div class="command-preview">
                  <span class="command-text">{$voiceCommand}</span>
                </div>
              {/if}
            </div>
          {:else if $voiceState === 'activated'}
            <div class="activated-state">
              <div class="activation-icon">
                <Sparkles size={32} />
              </div>
              <p class="activation-text">Voice activated!</p>
              <p class="keyword-text">Opening AI Chat...</p>
            </div>
          {:else}
            <div class="listening-state">
              <div class="waveform">
                <span class="wave-bar"></span>
                <span class="wave-bar"></span>
                <span class="wave-bar"></span>
                <span class="wave-bar"></span>
                <span class="wave-bar"></span>
              </div>
              <p class="listening-hint">Say "Hey Computer" to activate</p>
              {#if displayTranscript}
                <div class="transcript">
                  <span class="transcript-label">Heard:</span>
                  <span class="transcript-text">{displayTranscript}</span>
                </div>
              {/if}
            </div>
          {/if}
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .voice-activation {
    position: relative;
  }

  .voice-button {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    padding: 0;
    border-radius: 8px;
    color: #6b7280;
    background: transparent;
    border: none;
    cursor: pointer;
    transition: all 0.2s ease;
    overflow: visible;
  }

  .voice-button:hover {
    background: #f3f4f6;
    color: #6366f1;
  }

  .voice-button.listening {
    color: #10b981;
    background: #ecfdf5;
  }

  .voice-button.listening:hover {
    background: #d1fae5;
  }

  .voice-button.activated {
    color: #8b5cf6;
    background: #f5f3ff;
    animation: pulse-activated 0.5s ease;
  }

  .voice-button.error {
    color: #ef4444;
    background: #fef2f2;
  }

  .pulse-ring {
    position: absolute;
    width: 100%;
    height: 100%;
    border-radius: 8px;
    border: 2px solid #10b981;
    animation: pulse-ring 1.5s infinite;
    pointer-events: none;
  }

  @keyframes pulse-ring {
    0% {
      transform: scale(1);
      opacity: 1;
    }
    100% {
      transform: scale(1.5);
      opacity: 0;
    }
  }

  @keyframes pulse-activated {
    0%, 100% {
      transform: scale(1);
    }
    50% {
      transform: scale(1.1);
    }
  }

  .voice-panel {
    position: absolute;
    top: calc(100% + 0.5rem);
    right: 0;
    width: 280px;
    background: white;
    border-radius: 12px;
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.15), 0 2px 10px rgba(0, 0, 0, 0.1);
    overflow: hidden;
    z-index: 1000;
    animation: slide-down 0.2s ease;
  }

  @keyframes slide-down {
    from {
      opacity: 0;
      transform: translateY(-8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.75rem 1rem;
    background: linear-gradient(135deg, #10b981, #059669);
    color: white;
  }

  .voice-panel:has(.activated-state) .panel-header,
  .voice-panel:has(.capturing-state) .panel-header {
    background: linear-gradient(135deg, #8b5cf6, #7c3aed);
  }

  .voice-panel:has(.error-state) .panel-header {
    background: linear-gradient(135deg, #ef4444, #dc2626);
  }

  .panel-title {
    font-size: 0.875rem;
    font-weight: 600;
  }

  .close-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0.25rem;
    background: rgba(255, 255, 255, 0.2);
    border: none;
    border-radius: 4px;
    color: white;
    cursor: pointer;
    transition: background 0.2s;
  }

  .close-btn:hover {
    background: rgba(255, 255, 255, 0.3);
  }

  .panel-content {
    padding: 1rem;
  }

  /* Error state */
  .error-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
    text-align: center;
    color: #ef4444;
  }

  .error-state p {
    font-size: 0.875rem;
    color: #6b7280;
    margin: 0;
  }

  .retry-btn {
    padding: 0.5rem 1rem;
    font-size: 0.875rem;
    font-weight: 500;
    color: white;
    background: #6366f1;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: background 0.2s;
  }

  .retry-btn:hover {
    background: #4f46e5;
  }

  /* Listening state */
  .listening-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
  }

  .waveform {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    height: 32px;
  }

  .wave-bar {
    width: 4px;
    height: 100%;
    background: #10b981;
    border-radius: 2px;
    animation: wave 1s ease-in-out infinite;
  }

  .wave-bar:nth-child(1) { animation-delay: 0s; }
  .wave-bar:nth-child(2) { animation-delay: 0.1s; }
  .wave-bar:nth-child(3) { animation-delay: 0.2s; }
  .wave-bar:nth-child(4) { animation-delay: 0.3s; }
  .wave-bar:nth-child(5) { animation-delay: 0.4s; }

  @keyframes wave {
    0%, 100% {
      transform: scaleY(0.3);
    }
    50% {
      transform: scaleY(1);
    }
  }

  .listening-hint {
    font-size: 0.875rem;
    color: #6b7280;
    margin: 0;
  }

  .transcript {
    width: 100%;
    padding: 0.5rem;
    background: #f9fafb;
    border-radius: 6px;
    font-size: 0.75rem;
  }

  .transcript-label {
    color: #9ca3af;
    margin-right: 0.25rem;
  }

  .transcript-text {
    color: #4b5563;
    word-break: break-word;
  }

  /* Capturing state */
  .capturing-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
  }

  .waveform.capturing .wave-bar {
    background: #8b5cf6;
  }

  .capturing-hint {
    font-size: 0.875rem;
    color: #8b5cf6;
    font-weight: 500;
    margin: 0;
  }

  .command-preview {
    width: 100%;
    padding: 0.75rem;
    background: #f5f3ff;
    border-radius: 8px;
    border: 1px solid #e9d5ff;
  }

  .command-text {
    font-size: 0.875rem;
    color: #6b21a8;
    word-break: break-word;
  }

  /* Activated state */
  .activated-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
    text-align: center;
  }

  .activation-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 56px;
    background: linear-gradient(135deg, #f5f3ff, #ede9fe);
    border-radius: 50%;
    color: #8b5cf6;
    animation: bounce 0.5s ease;
  }

  @keyframes bounce {
    0%, 100% {
      transform: scale(1);
    }
    50% {
      transform: scale(1.2);
    }
  }

  .activation-text {
    font-size: 0.875rem;
    font-weight: 600;
    color: #8b5cf6;
    margin: 0;
  }

  .keyword-text {
    font-size: 0.75rem;
    color: #6b7280;
    margin: 0;
    font-style: italic;
  }

  .context-hint {
    font-size: 0.625rem;
    color: #9ca3af;
    margin: 0;
    padding-top: 0.5rem;
    border-top: 1px solid #e5e7eb;
    width: 100%;
  }

  @media (max-width: 480px) {
    .voice-panel {
      width: calc(100vw - 2rem);
      right: -0.5rem;
    }
  }
</style>
