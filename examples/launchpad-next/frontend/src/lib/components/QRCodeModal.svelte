<script lang="ts">
  import { onMount } from 'svelte';
  import { X, Copy, Check, RefreshCw, Smartphone, ExternalLink } from 'lucide-svelte';
  import QRCode from 'qrcode';
  import { getSession } from '$lib/raisin';

  interface Props {
    boardId: string;
    onClose: () => void;
  }

  let { boardId, onClose }: Props = $props();

  let qrCodeDataUrl = $state<string | null>(null);
  let networkIp = $state<string | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let copied = $state(false);
  let oculusCopied = $state(false);
  let manualIp = $state('');
  let showManualInput = $state(false);

  // Build AR URL with optional JWT token for Quest browser authentication
  function buildArUrl(ip: string, board: string): string {
    const baseUrl = `http://${ip}:5173/tasks/${board}/ar`;
    const session = getSession();
    return session?.accessToken
      ? `${baseUrl}?token=${encodeURIComponent(session.accessToken)}`
      : baseUrl;
  }

  const arUrl = $derived(
    networkIp
      ? buildArUrl(networkIp, boardId)
      : null
  );

  // Oculus deep link to open URL directly in Quest browser
  const oculusDeepLink = $derived(
    arUrl
      ? `https://www.oculus.com/open_url/?url=${encodeURIComponent(arUrl)}`
      : null
  );

  onMount(async () => {
    await fetchNetworkIp();
  });

  async function fetchNetworkIp() {
    loading = true;
    error = null;

    try {
      const response = await fetch('/api/network-ip');
      const data = await response.json();

      if (data.preferred) {
        networkIp = data.preferred;
        await generateQRCode(buildArUrl(data.preferred, boardId));
      } else {
        error = 'Could not detect network IP address';
        showManualInput = true;
      }
    } catch (e) {
      error = 'Failed to fetch network IP';
      showManualInput = true;
    } finally {
      loading = false;
    }
  }

  async function generateQRCode(url: string) {
    try {
      qrCodeDataUrl = await QRCode.toDataURL(url, {
        width: 280,
        margin: 2,
        color: {
          dark: '#1e293b',
          light: '#ffffff'
        }
      });
    } catch (e) {
      error = 'Failed to generate QR code';
    }
  }

  async function applyManualIp() {
    if (!manualIp.trim()) return;

    networkIp = manualIp.trim();
    showManualInput = false;
    await generateQRCode(buildArUrl(manualIp.trim(), boardId));
  }

  async function copyUrl() {
    if (!arUrl) return;

    try {
      await navigator.clipboard.writeText(arUrl);
      copied = true;
      setTimeout(() => {
        copied = false;
      }, 2000);
    } catch (e) {
      console.error('Failed to copy URL:', e);
    }
  }

  async function copyOculusLink() {
    if (!oculusDeepLink) return;

    try {
      await navigator.clipboard.writeText(oculusDeepLink);
      oculusCopied = true;
      setTimeout(() => {
        oculusCopied = false;
      }, 2000);
    } catch (e) {
      console.error('Failed to copy Oculus link:', e);
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      onClose();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="modal-overlay" onclick={onClose}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="modal" onclick={(e) => e.stopPropagation()}>
    <div class="modal-header">
      <div class="header-icon">
        <Smartphone size={20} />
      </div>
      <div class="header-text">
        <h2>Open in AR</h2>
        <p>Scan with your Meta Quest browser</p>
      </div>
      <button class="close-btn" onclick={onClose}>
        <X size={20} />
      </button>
    </div>

    <div class="modal-body">
      {#if loading}
        <div class="loading-state">
          <div class="spinner"></div>
          <p>Detecting network address...</p>
        </div>
      {:else if showManualInput}
        <div class="manual-input">
          <label for="manual-ip">Enter your computer's IP address:</label>
          <div class="input-row">
            <input
              id="manual-ip"
              type="text"
              placeholder="192.168.1.100"
              bind:value={manualIp}
              onkeydown={(e) => e.key === 'Enter' && applyManualIp()}
            />
            <button class="apply-btn" onclick={applyManualIp}>
              Apply
            </button>
          </div>
          <p class="hint">
            Find your IP in System Preferences &rarr; Network (Mac) or
            Settings &rarr; Network (Windows)
          </p>
        </div>
      {:else if qrCodeDataUrl}
        <div class="qr-container">
          <img src={qrCodeDataUrl} alt="QR Code for AR view" class="qr-code" />
        </div>

        <div class="url-display">
          <code>{arUrl}</code>
          <button class="copy-btn" onclick={copyUrl} title="Copy URL">
            {#if copied}
              <Check size={16} />
            {:else}
              <Copy size={16} />
            {/if}
          </button>
        </div>

        <div class="oculus-link-section">
          <div class="oculus-link-header">
            <ExternalLink size={16} />
            <span>Quest Direct Link</span>
          </div>
          <p class="oculus-hint">
            Click this link on any device to open directly in your Quest browser
          </p>
          <div class="url-display oculus">
            <code>{oculusDeepLink}</code>
            <button class="copy-btn" onclick={copyOculusLink} title="Copy Oculus link">
              {#if oculusCopied}
                <Check size={16} />
              {:else}
                <Copy size={16} />
              {/if}
            </button>
          </div>
          <a href={oculusDeepLink} class="oculus-open-btn" target="_blank" rel="noopener noreferrer">
            <ExternalLink size={16} />
            Open in Quest Browser
          </a>
        </div>

        <button class="change-ip-btn" onclick={() => showManualInput = true}>
          <RefreshCw size={14} />
          Use different IP address
        </button>
      {/if}

      {#if error && !showManualInput}
        <div class="error-message">
          <p>{error}</p>
          <button onclick={() => showManualInput = true}>Enter IP manually</button>
        </div>
      {/if}
    </div>

    <div class="modal-footer">
      <div class="instructions">
        <h4>How to use:</h4>
        <ol>
          <li>Put on your Meta Quest headset</li>
          <li>Open the Quest Browser and scan the QR code, <strong>or</strong></li>
          <li>Click "Open in Quest Browser" from your phone/computer</li>
          <li>Tap "Enter AR" when the page loads</li>
        </ol>
      </div>
    </div>
  </div>
</div>

<style>
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    padding: 1rem;
    backdrop-filter: blur(4px);
  }

  .modal {
    background: white;
    border-radius: 1rem;
    width: 100%;
    max-width: 440px;
    max-height: 90vh;
    overflow-y: auto;
    box-shadow: 0 20px 40px rgba(0, 0, 0, 0.3);
  }

  .modal-header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 1.25rem 1.5rem;
    background: linear-gradient(135deg, #8b5cf6 0%, #7c3aed 100%);
    color: white;
  }

  .header-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 40px;
    height: 40px;
    background: rgba(255, 255, 255, 0.2);
    border-radius: 0.5rem;
  }

  .header-text {
    flex: 1;
  }

  .header-text h2 {
    font-size: 1.125rem;
    font-weight: 600;
    margin: 0;
  }

  .header-text p {
    font-size: 0.75rem;
    margin: 0.25rem 0 0;
    opacity: 0.9;
  }

  .close-btn {
    background: rgba(255, 255, 255, 0.1);
    border: none;
    color: white;
    width: 36px;
    height: 36px;
    border-radius: 0.5rem;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s;
  }

  .close-btn:hover {
    background: rgba(255, 255, 255, 0.2);
  }

  .modal-body {
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
  }

  .loading-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1rem;
    padding: 2rem 0;
  }

  .spinner {
    width: 40px;
    height: 40px;
    border: 3px solid #e2e8f0;
    border-top-color: #8b5cf6;
    border-radius: 50%;
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  .loading-state p {
    color: #64748b;
    font-size: 0.875rem;
    margin: 0;
  }

  .qr-container {
    background: #f8fafc;
    padding: 1rem;
    border-radius: 0.75rem;
    border: 1px solid #e2e8f0;
  }

  .qr-code {
    display: block;
    width: 280px;
    height: 280px;
  }

  .url-display {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.75rem;
    background: #f1f5f9;
    border-radius: 0.5rem;
  }

  .url-display code {
    flex: 1;
    font-size: 0.75rem;
    color: #475569;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .copy-btn {
    background: white;
    border: 1px solid #e2e8f0;
    color: #64748b;
    width: 32px;
    height: 32px;
    border-radius: 0.375rem;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: color 0.15s, border-color 0.15s;
    flex-shrink: 0;
  }

  .copy-btn:hover {
    color: #8b5cf6;
    border-color: #8b5cf6;
  }

  .change-ip-btn {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: none;
    border: none;
    color: #64748b;
    font-size: 0.75rem;
    cursor: pointer;
    padding: 0.5rem;
  }

  .change-ip-btn:hover {
    color: #8b5cf6;
  }

  .oculus-link-section {
    width: 100%;
    padding: 1rem;
    background: #f8fafc;
    border-radius: 0.75rem;
    border: 1px solid #e2e8f0;
  }

  .oculus-link-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.875rem;
    font-weight: 600;
    color: #1e293b;
    margin-bottom: 0.25rem;
  }

  .oculus-hint {
    font-size: 0.75rem;
    color: #64748b;
    margin: 0 0 0.75rem;
  }

  .url-display.oculus {
    background: #e2e8f0;
    margin-bottom: 0.75rem;
  }

  .url-display.oculus code {
    font-size: 0.65rem;
  }

  .oculus-open-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.625rem 1rem;
    background: linear-gradient(135deg, #0ea5e9 0%, #0284c7 100%);
    color: white;
    border: none;
    border-radius: 0.5rem;
    font-size: 0.875rem;
    font-weight: 500;
    text-decoration: none;
    cursor: pointer;
    transition: opacity 0.15s;
  }

  .oculus-open-btn:hover {
    opacity: 0.9;
  }

  .manual-input {
    width: 100%;
    padding: 1rem 0;
  }

  .manual-input label {
    display: block;
    font-size: 0.875rem;
    font-weight: 500;
    color: #374151;
    margin-bottom: 0.5rem;
  }

  .input-row {
    display: flex;
    gap: 0.5rem;
  }

  .input-row input {
    flex: 1;
    padding: 0.625rem 0.75rem;
    border: 1px solid #e2e8f0;
    border-radius: 0.5rem;
    font-size: 0.875rem;
  }

  .input-row input:focus {
    outline: none;
    border-color: #8b5cf6;
    box-shadow: 0 0 0 3px rgba(139, 92, 246, 0.1);
  }

  .apply-btn {
    padding: 0.625rem 1rem;
    background: #8b5cf6;
    color: white;
    border: none;
    border-radius: 0.5rem;
    font-size: 0.875rem;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.15s;
  }

  .apply-btn:hover {
    background: #7c3aed;
  }

  .hint {
    font-size: 0.75rem;
    color: #94a3b8;
    margin: 0.75rem 0 0;
  }

  .error-message {
    text-align: center;
    padding: 1rem;
    color: #ef4444;
  }

  .error-message p {
    margin: 0 0 0.75rem;
  }

  .error-message button {
    background: none;
    border: none;
    color: #8b5cf6;
    font-size: 0.875rem;
    cursor: pointer;
    text-decoration: underline;
  }

  .modal-footer {
    padding: 1rem 1.5rem;
    background: #f8fafc;
    border-top: 1px solid #e2e8f0;
  }

  .instructions h4 {
    font-size: 0.75rem;
    font-weight: 600;
    color: #475569;
    margin: 0 0 0.5rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .instructions ol {
    margin: 0;
    padding-left: 1.25rem;
    font-size: 0.8rem;
    color: #64748b;
    line-height: 1.6;
  }

  .instructions li {
    margin-bottom: 0.25rem;
  }
</style>
