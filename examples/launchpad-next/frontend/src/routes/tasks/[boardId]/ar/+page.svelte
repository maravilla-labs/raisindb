<script lang="ts">
  import { onMount } from 'svelte';
  import { browser } from '$app/environment';
  import type { PageNode } from '$lib/raisin';
  import XRCanvas from '$lib/components/xr/XRCanvas.svelte';
  import XRFallback from '$lib/components/xr/XRFallback.svelte';
  import { ArrowLeft, Smartphone } from 'lucide-svelte';

  interface Props {
    data: {
      board: PageNode;
      boardId: string;
    };
  }

  let { data }: Props = $props();

  let isXRSupported = $state(false);
  let isXRActive = $state(false);
  let checkingSupport = $state(true);
  let xrMode = $state<'immersive-ar' | 'immersive-vr' | null>(null);
  let xrError = $state<string | null>(null);

  onMount(async () => {
    if (!browser) return;

    // Check if we're on HTTPS (required for WebXR)
    if (location.protocol !== 'https:' && location.hostname !== 'localhost') {
      console.warn('[WebXR] HTTPS required. Current protocol:', location.protocol);
      xrError = 'WebXR requires HTTPS. Please access this page via https://';
      checkingSupport = false;
      return;
    }

    // Check for WebXR support
    if (!('xr' in navigator)) {
      console.warn('[WebXR] navigator.xr not available');
      xrError = 'WebXR API not available in this browser';
      checkingSupport = false;
      return;
    }

    const xr = (navigator as any).xr;

    try {
      // First try immersive-ar (passthrough AR on Quest)
      const arSupported = await xr.isSessionSupported('immersive-ar');
      console.log('[WebXR] immersive-ar supported:', arSupported);

      if (arSupported) {
        isXRSupported = true;
        xrMode = 'immersive-ar';
      } else {
        // Fall back to immersive-vr (standard Quest VR mode)
        const vrSupported = await xr.isSessionSupported('immersive-vr');
        console.log('[WebXR] immersive-vr supported:', vrSupported);

        if (vrSupported) {
          isXRSupported = true;
          xrMode = 'immersive-vr';
        } else {
          xrError = 'Neither AR nor VR mode supported. Enable WebXR in Quest settings.';
        }
      }
    } catch (e) {
      console.warn('[WebXR] Support check failed:', e);
      xrError = `WebXR check failed: ${e instanceof Error ? e.message : 'Unknown error'}`;
      isXRSupported = false;
    }

    checkingSupport = false;
  });

  function handleXRStateChange(active: boolean) {
    isXRActive = active;
  }
</script>

<svelte:head>
  <title>{data.board.properties.title} - AR View</title>
</svelte:head>

<div class="ar-page" class:xr-active={isXRActive}>
  {#if !isXRActive}
    <header class="ar-header">
      <a href="/tasks/{data.boardId}" class="back-link">
        <ArrowLeft size={20} />
        <span>Back to 2D View</span>
      </a>
      <h1>{data.board.properties.title}</h1>
      <div class="ar-badge">
        <Smartphone size={16} />
        AR Mode
      </div>
    </header>
  {/if}

  <main class="ar-content">
    {#if checkingSupport}
      <div class="loading-state">
        <div class="spinner"></div>
        <p>Checking WebXR support...</p>
      </div>
    {:else if !isXRSupported}
      <XRFallback boardId={data.boardId} errorMessage={xrError} />
    {:else}
      <XRCanvas
        board={data.board}
        {xrMode}
        onXRStateChange={handleXRStateChange}
      />

      {#if !isXRActive}
        <div class="xr-instructions">
          <h2>Ready for {xrMode === 'immersive-vr' ? 'VR' : 'Mixed Reality'}</h2>
          <p>
            {#if xrMode === 'immersive-vr'}
              Put on your Quest headset and tap "Enter VR" to view your
              kanban board in virtual reality.
            {:else}
              Put on your Quest headset and tap "Enter AR" to view your
              kanban board in mixed reality with passthrough.
            {/if}
          </p>
          <ul class="requirements">
            <li>Meta Quest headset required</li>
            <li>Hand tracking must be enabled in Settings</li>
            <li>Pinch to grab and move cards</li>
            {#if xrMode === 'immersive-ar'}
              <li>Passthrough enabled for AR mode</li>
            {/if}
          </ul>
          <p class="mode-badge">Mode: <code>{xrMode}</code></p>
        </div>
      {/if}
    {/if}
  </main>
</div>

<style>
  .ar-page {
    min-height: 100vh;
    background: linear-gradient(135deg, #1e1b4b 0%, #312e81 50%, #4c1d95 100%);
    color: white;
    display: flex;
    flex-direction: column;
  }

  .ar-page.xr-active {
    background: transparent;
  }

  .ar-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 1rem 2rem;
    background: rgba(0, 0, 0, 0.2);
    backdrop-filter: blur(10px);
  }

  .back-link {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: rgba(255, 255, 255, 0.8);
    text-decoration: none;
    font-size: 0.875rem;
    transition: color 0.15s;
  }

  .back-link:hover {
    color: white;
  }

  .ar-header h1 {
    font-size: 1.25rem;
    font-weight: 600;
    margin: 0;
  }

  .ar-badge {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 1rem;
    background: rgba(139, 92, 246, 0.3);
    border: 1px solid rgba(139, 92, 246, 0.5);
    border-radius: 2rem;
    font-size: 0.75rem;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .ar-content {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 2rem;
    position: relative;
  }

  .loading-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
  }

  .spinner {
    width: 48px;
    height: 48px;
    border: 3px solid rgba(255, 255, 255, 0.2);
    border-top-color: white;
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .xr-instructions {
    max-width: 480px;
    text-align: center;
    padding: 2rem;
    background: rgba(255, 255, 255, 0.1);
    border-radius: 1rem;
    backdrop-filter: blur(10px);
    margin-bottom: 2rem;
  }

  .xr-instructions h2 {
    font-size: 1.5rem;
    font-weight: 600;
    margin: 0 0 1rem;
  }

  .xr-instructions p {
    color: rgba(255, 255, 255, 0.8);
    line-height: 1.6;
    margin: 0 0 1.5rem;
  }

  .requirements {
    list-style: none;
    padding: 0;
    margin: 0;
    text-align: left;
  }

  .requirements li {
    position: relative;
    padding-left: 1.5rem;
    margin-bottom: 0.5rem;
    color: rgba(255, 255, 255, 0.7);
    font-size: 0.875rem;
  }

  .requirements li::before {
    content: '•';
    position: absolute;
    left: 0;
    color: #8b5cf6;
  }

  .mode-badge {
    margin-top: 1rem;
    font-size: 0.75rem;
    color: rgba(255, 255, 255, 0.5);
  }

  .mode-badge code {
    background: rgba(139, 92, 246, 0.3);
    padding: 0.125rem 0.5rem;
    border-radius: 0.25rem;
    font-size: 0.75rem;
  }
</style>
