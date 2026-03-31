<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { browser } from '$app/environment';
  import * as THREE from 'three';
  import type { PageNode } from '$lib/raisin';
  import { XRScene } from '$lib/xr/scene';
  import { HandTracker } from '$lib/xr/hand-tracker';
  import { HandInteraction } from '$lib/xr/hand-interaction';
  import { BoardSync } from '$lib/xr/board-sync';
  import { xrStore } from '$lib/stores/xr-state';
  import XRControls from './XRControls.svelte';

  interface Props {
    board: PageNode;
    xrMode?: 'immersive-ar' | 'immersive-vr' | null;
    onXRStateChange?: (active: boolean) => void;
  }

  let { board, xrMode = 'immersive-ar', onXRStateChange }: Props = $props();

  let containerRef: HTMLDivElement | null = $state(null);
  let xrScene: XRScene | null = null;
  let handTracker: HandTracker | null = null;
  let handInteraction: HandInteraction | null = null;
  let boardSync: BoardSync | null = null;

  let isLoading = $state(false);
  let error = $state<string | null>(null);
  let isXRActive = $state(false);

  onMount(() => {
    if (!browser || !containerRef) return;
    initializeScene();
  });

  onDestroy(() => {
    cleanup();
  });

  function initializeScene() {
    if (!containerRef) return;

    try {
      // Create XR scene
      xrScene = new XRScene(containerRef);

      // Initialize board sync
      boardSync = new BoardSync(board, xrScene.getBoardGroup());
      boardSync.initialize();

      // Start render loop (non-XR preview)
      xrScene.startRenderLoop();

    } catch (e) {
      console.error('Failed to initialize XR scene:', e);
      error = 'Failed to initialize 3D scene';
    }
  }

  async function enterXR() {
    if (!xrScene || !browser) return;

    isLoading = true;
    error = null;

    try {
      const xr = (navigator as any).xr;
      if (!xr) {
        throw new Error('WebXR not supported');
      }

      // Request XR session with hand tracking
      const mode = xrMode || 'immersive-ar';
      console.log(`[XR] Requesting session: ${mode}`);

      const sessionInit: XRSessionInit = {
        requiredFeatures: ['local-floor', 'hand-tracking'],
        optionalFeatures: mode === 'immersive-ar'
          ? ['plane-detection', 'mesh-detection', 'bounded-floor']
          : ['bounded-floor']
      };

      const session = await xr.requestSession(mode, sessionInit);

      // Configure renderer for XR
      const renderer = xrScene.getRenderer();
      await renderer.xr.setSession(session);

      // Setup hand tracking
      handTracker = new HandTracker(renderer, xrScene.getScene());
      handTracker.setup();

      handTracker.onHandConnected = (side) => {
        xrStore.setHandConnected(side, true);
        console.log(`[XR] ${side} hand connected`);
      };

      handTracker.onHandDisconnected = (side) => {
        xrStore.setHandConnected(side, false);
        console.log(`[XR] ${side} hand disconnected`);
      };

      // Setup hand interaction (pointing + zoom)
      handInteraction = new HandInteraction(
        handTracker,
        xrScene.getScene(),
        xrScene.getBoardGroup()
      );

      // Connect to board sync for card hover/selection
      if (boardSync) {
        boardSync.setHandInteraction(handInteraction);
      }

      // Position board in front of user after a short delay
      // (wait for XR camera to initialize)
      const positionBoard = () => {
        if (!xrScene || !renderer.xr.isPresenting) return;

        const camera = renderer.xr.getCamera();
        const cameraPos = new THREE.Vector3();
        camera.getWorldPosition(cameraPos);

        // Get camera's forward direction (where user is looking)
        const forward = new THREE.Vector3(0, 0, -1);
        forward.applyQuaternion(camera.quaternion);
        forward.y = 0; // Keep board level
        forward.normalize();

        // Position board 0.8m in front of user at eye level
        const boardPos = cameraPos.clone().add(forward.multiplyScalar(0.8));
        boardPos.y = cameraPos.y - 0.2; // Slightly below eye level

        // Make board face the user
        const boardGroup = xrScene.getBoardGroup();
        boardGroup.position.copy(boardPos);
        boardGroup.lookAt(cameraPos.x, boardPos.y, cameraPos.z);

        console.log('[XR] Board positioned at:', boardPos);
      };

      // Try to position immediately, then again after delay
      setTimeout(positionBoard, 100);
      setTimeout(positionBoard, 500);

      // Start XR render loop
      xrScene.startRenderLoop((time, frame) => {
        // Update hand tracking
        if (handTracker && frame) {
          handTracker.update(frame);
        }

        // Update hand interaction (pointing, hover, zoom)
        if (handInteraction) {
          handInteraction.update();
        }
      });

      // Listen for session end
      session.addEventListener('end', () => {
        handleSessionEnd();
      });

      // Update state
      isXRActive = true;
      xrStore.setSession(session);
      onXRStateChange?.(true);

    } catch (e) {
      console.error('Failed to start XR session:', e);
      const message = e instanceof Error ? e.message : 'Unknown error';

      if (message.includes('NotAllowedError') || message.includes('denied')) {
        error = 'Permission denied. Please allow access to start AR.';
      } else if (message.includes('NotSupportedError')) {
        error = 'AR mode not supported on this device.';
      } else {
        error = `Failed to start AR: ${message}`;
      }
    } finally {
      isLoading = false;
    }
  }

  function exitXR() {
    const renderer = xrScene?.getRenderer();
    const session = renderer?.xr.getSession();

    if (session) {
      session.end();
    }
  }

  function handleSessionEnd() {
    // Clean up hand interaction
    if (handInteraction) {
      handInteraction.dispose();
      handInteraction = null;
    }

    // Clean up hand tracking
    if (handTracker) {
      handTracker.dispose();
      handTracker = null;
    }

    // Reset state
    isXRActive = false;
    xrStore.reset();
    onXRStateChange?.(false);

    // Restart non-XR preview
    if (xrScene) {
      xrScene.startRenderLoop();
    }
  }

  function cleanup() {
    // End any active XR session
    exitXR();

    // Clean up board sync
    if (boardSync) {
      boardSync.dispose();
      boardSync = null;
    }

    // Clean up scene
    if (xrScene) {
      xrScene.dispose();
      xrScene = null;
    }
  }
</script>

<div class="xr-canvas-wrapper">
  <div class="canvas-container" bind:this={containerRef}></div>

  <XRControls
    isActive={isXRActive}
    {isLoading}
    {error}
    onEnterXR={enterXR}
    onExitXR={exitXR}
  />

  {#if isXRActive}
    <div class="xr-active-indicator">
      <span class="pulse"></span>
      AR Active
    </div>
  {/if}
</div>

<style>
  .xr-canvas-wrapper {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 400px;
  }

  .canvas-container {
    width: 100%;
    height: 100%;
    min-height: 400px;
  }

  .canvas-container :global(canvas) {
    display: block;
    width: 100% !important;
    height: 100% !important;
  }

  .xr-active-indicator {
    position: fixed;
    top: 1rem;
    left: 1rem;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 1rem;
    background: rgba(16, 185, 129, 0.9);
    color: white;
    font-size: 0.75rem;
    font-weight: 600;
    border-radius: 2rem;
    backdrop-filter: blur(10px);
  }

  .pulse {
    width: 8px;
    height: 8px;
    background: white;
    border-radius: 50%;
    animation: pulse 2s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 0.5; transform: scale(0.8); }
    50% { opacity: 1; transform: scale(1.2); }
  }
</style>
