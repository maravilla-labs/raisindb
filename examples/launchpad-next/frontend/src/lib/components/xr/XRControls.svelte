<script lang="ts">
  import { Play, Square, Hand, RefreshCw } from 'lucide-svelte';

  interface Props {
    isActive: boolean;
    onEnterXR: () => void;
    onExitXR: () => void;
    isLoading?: boolean;
    error?: string | null;
  }

  let {
    isActive,
    onEnterXR,
    onExitXR,
    isLoading = false,
    error = null
  }: Props = $props();
</script>

<div class="xr-controls" class:active={isActive}>
  {#if error}
    <div class="error-banner">
      <p>{error}</p>
    </div>
  {/if}

  <div class="controls-inner">
    {#if !isActive}
      <button
        class="enter-btn"
        onclick={onEnterXR}
        disabled={isLoading}
      >
        {#if isLoading}
          <RefreshCw size={20} class="spinning" />
          <span>Starting AR...</span>
        {:else}
          <Play size={20} />
          <span>Enter AR</span>
        {/if}
      </button>

      <div class="hint">
        <Hand size={16} />
        <span>Use pinch gesture to grab cards</span>
      </div>
    {:else}
      <button
        class="exit-btn"
        onclick={onExitXR}
      >
        <Square size={20} />
        <span>Exit AR</span>
      </button>
    {/if}
  </div>
</div>

<style>
  .xr-controls {
    position: fixed;
    bottom: 2rem;
    left: 50%;
    transform: translateX(-50%);
    z-index: 100;
  }

  .xr-controls.active {
    bottom: auto;
    top: 1rem;
  }

  .controls-inner {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
  }

  .error-banner {
    background: rgba(239, 68, 68, 0.9);
    color: white;
    padding: 0.75rem 1.5rem;
    border-radius: 0.5rem;
    font-size: 0.875rem;
    margin-bottom: 0.5rem;
    backdrop-filter: blur(10px);
  }

  .error-banner p {
    margin: 0;
  }

  .enter-btn,
  .exit-btn {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 1rem 2rem;
    font-size: 1rem;
    font-weight: 600;
    border: none;
    border-radius: 2rem;
    cursor: pointer;
    transition: transform 0.15s, box-shadow 0.15s;
  }

  .enter-btn {
    background: linear-gradient(135deg, #8b5cf6 0%, #7c3aed 100%);
    color: white;
    box-shadow: 0 4px 20px rgba(139, 92, 246, 0.4);
  }

  .enter-btn:hover:not(:disabled) {
    transform: scale(1.05);
    box-shadow: 0 6px 24px rgba(139, 92, 246, 0.5);
  }

  .enter-btn:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .exit-btn {
    background: rgba(255, 255, 255, 0.9);
    color: #1e293b;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
  }

  .exit-btn:hover {
    background: white;
  }

  .hint {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 1rem;
    background: rgba(255, 255, 255, 0.1);
    border-radius: 1rem;
    color: rgba(255, 255, 255, 0.8);
    font-size: 0.75rem;
    backdrop-filter: blur(10px);
  }

  :global(.spinning) {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
