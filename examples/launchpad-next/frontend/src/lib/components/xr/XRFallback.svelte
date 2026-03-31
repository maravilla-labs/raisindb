<script lang="ts">
  import { AlertTriangle, Smartphone, ExternalLink } from 'lucide-svelte';

  interface Props {
    boardId: string;
    errorMessage?: string | null;
  }

  let { boardId, errorMessage }: Props = $props();
</script>

<div class="fallback">
  <div class="fallback-content">
    <div class="icon">
      <AlertTriangle size={48} />
    </div>

    <h2>WebXR Not Available</h2>

    <p class="message">
      This browser doesn't support WebXR AR features required for the mixed reality experience.
    </p>

    {#if errorMessage}
      <div class="error-detail">
        <code>{errorMessage}</code>
      </div>
    {/if}

    <div class="requirements">
      <h3>
        <Smartphone size={18} />
        Requirements
      </h3>
      <ul>
        <li>Meta Quest 2, Quest Pro, or Quest 3</li>
        <li>Quest Browser (built-in)</li>
        <li>Hand tracking enabled in Settings</li>
      </ul>
    </div>

    <div class="instructions">
      <h3>To view in AR:</h3>
      <ol>
        <li>Open the Quest Browser on your headset</li>
        <li>Navigate to this URL or scan the QR code</li>
        <li>Tap "Enter AR" when prompted</li>
      </ol>
    </div>

    <a href="/tasks/{boardId}" class="back-link">
      <ExternalLink size={16} />
      Return to 2D Board
    </a>
  </div>
</div>

<style>
  .fallback {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem;
    width: 100%;
    max-width: 500px;
  }

  .fallback-content {
    background: rgba(255, 255, 255, 0.1);
    border: 1px solid rgba(255, 255, 255, 0.2);
    border-radius: 1rem;
    padding: 2rem;
    text-align: center;
    backdrop-filter: blur(10px);
  }

  .icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 80px;
    height: 80px;
    background: rgba(251, 191, 36, 0.2);
    border-radius: 50%;
    color: #fbbf24;
    margin-bottom: 1.5rem;
  }

  h2 {
    font-size: 1.5rem;
    font-weight: 600;
    margin: 0 0 1rem;
  }

  .message {
    color: rgba(255, 255, 255, 0.8);
    line-height: 1.6;
    margin: 0 0 1.5rem;
  }

  .error-detail {
    background: rgba(239, 68, 68, 0.2);
    border: 1px solid rgba(239, 68, 68, 0.4);
    border-radius: 0.5rem;
    padding: 0.75rem 1rem;
    margin-bottom: 1.5rem;
  }

  .error-detail code {
    font-size: 0.75rem;
    color: #fca5a5;
    word-break: break-all;
  }

  .requirements,
  .instructions {
    background: rgba(0, 0, 0, 0.2);
    border-radius: 0.75rem;
    padding: 1rem;
    margin-bottom: 1rem;
    text-align: left;
  }

  .requirements h3,
  .instructions h3 {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.875rem;
    font-weight: 600;
    margin: 0 0 0.75rem;
    color: rgba(255, 255, 255, 0.9);
  }

  .requirements ul,
  .instructions ol {
    margin: 0;
    padding-left: 1.25rem;
    font-size: 0.875rem;
    color: rgba(255, 255, 255, 0.7);
    line-height: 1.6;
  }

  .requirements li,
  .instructions li {
    margin-bottom: 0.25rem;
  }

  .back-link {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1.5rem;
    background: rgba(139, 92, 246, 0.3);
    border: 1px solid rgba(139, 92, 246, 0.5);
    border-radius: 0.5rem;
    color: white;
    text-decoration: none;
    font-size: 0.875rem;
    font-weight: 500;
    transition: background 0.15s;
    margin-top: 0.5rem;
  }

  .back-link:hover {
    background: rgba(139, 92, 246, 0.4);
  }
</style>
