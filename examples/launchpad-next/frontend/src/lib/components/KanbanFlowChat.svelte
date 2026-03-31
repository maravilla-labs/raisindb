<script lang="ts">
  import { Send, X, Bot, Loader2 } from 'lucide-svelte';

  interface ChatMessage { role: string; content: string; }

  interface Props {
    cardTitle: string;
    messages: ChatMessage[];
    isStreaming: boolean;
    streamingText: string;
    error: string | null;
    onSendMessage: (content: string) => Promise<void>;
    onDismiss: () => void;
  }

  let { cardTitle, messages, isStreaming, streamingText, error, onSendMessage, onDismiss }: Props = $props();

  let messageInput = $state('');
  let sending = $state(false);
  let messagesContainer: HTMLDivElement | null = $state(null);

  $effect(() => {
    if (messages.length > 0 && messagesContainer) {
      setTimeout(() => { messagesContainer?.scrollTo(0, messagesContainer.scrollHeight); }, 50);
    }
  });

  async function handleSend() {
    if (!messageInput.trim() || sending || isStreaming) return;
    sending = true;
    const content = messageInput.trim();
    messageInput = '';
    try { await onSendMessage(content); }
    finally { sending = false; }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleSend(); }
  }
</script>

<div class="panel">
  <div class="hdr">
    <div class="hdr-info">
      <Bot size={18} />
      <div>
        <span class="title">AI Summary Assistant</span>
        <span class="subtitle">Task: {cardTitle}</span>
      </div>
    </div>
    <button class="close" onclick={() => onDismiss()}><X size={18} /></button>
  </div>

  <div class="msgs" bind:this={messagesContainer}>
    {#each messages as msg, i (i)}
      <div class="msg" class:user={msg.role === 'user'} class:asst={msg.role === 'assistant'}>
        {#if msg.role === 'assistant'}
          <div class="avatar"><Bot size={14} /></div>
        {/if}
        <div class="bubble">{msg.content}</div>
      </div>
    {/each}

    {#if isStreaming && streamingText}
      <div class="msg asst">
        <div class="avatar"><Bot size={14} /></div>
        <div class="bubble streaming">{streamingText}<span class="cursor"></span></div>
      </div>
    {:else if isStreaming}
      <div class="typing">
        <Bot size={14} />
        <span class="dots"><span></span><span></span><span></span></span>
      </div>
    {/if}

    {#if error}<div class="err">{error}</div>{/if}
  </div>

  <div class="input">
    <textarea bind:value={messageInput} onkeydown={handleKeydown}
      placeholder="Reply to the assistant..." disabled={sending || isStreaming} rows="1"></textarea>
    <button class="send" onclick={handleSend} disabled={!messageInput.trim() || sending || isStreaming}>
      {#if sending || isStreaming}<Loader2 size={16} class="spin" />{:else}<Send size={16} />{/if}
    </button>
  </div>
</div>

<style>
  .panel { position: fixed; bottom: 1rem; right: 1rem; width: 380px; max-height: 520px; background: var(--color-bg-card); border: 1px solid var(--color-border); border-radius: var(--radius-md); box-shadow: 0 8px 30px rgba(0,0,0,0.5); display: flex; flex-direction: column; z-index: 200; animation: up 0.25s ease-out; }
  @keyframes up { from { opacity: 0; transform: translateY(20px); } to { opacity: 1; transform: translateY(0); } }
  .hdr { display: flex; align-items: center; justify-content: space-between; padding: 0.75rem 1rem; background: var(--color-bg-elevated); border-bottom: 1px solid var(--color-border); border-radius: var(--radius-md) var(--radius-md) 0 0; color: var(--color-success); }
  .hdr-info { display: flex; align-items: center; gap: 0.625rem; min-width: 0; }
  .title { display: block; font-weight: 600; font-size: 0.875rem; color: var(--color-text-heading); }
  .subtitle { display: block; font-size: 0.6875rem; color: var(--color-text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 220px; }
  .close { padding: 0.375rem; background: var(--color-surface); border: 1px solid var(--color-border); border-radius: var(--radius-sm); color: var(--color-text-muted); cursor: pointer; display: flex; align-items: center; transition: color 0.2s, border-color 0.2s; }
  .close:hover { border-color: var(--color-text-muted); color: var(--color-text); }
  .msgs { flex: 1; overflow-y: auto; padding: 0.75rem; display: flex; flex-direction: column; gap: 0.625rem; min-height: 200px; max-height: 340px; background: var(--color-bg); }
  .msg { display: flex; gap: 0.5rem; align-items: flex-start; }
  .msg.user { justify-content: flex-end; }
  .avatar { width: 28px; height: 28px; background: var(--color-success); border-radius: 50%; display: flex; align-items: center; justify-content: center; color: var(--color-bg); flex-shrink: 0; }
  .bubble { max-width: 75%; padding: 0.5rem 0.75rem; border-radius: 0.75rem; font-size: 0.8125rem; line-height: 1.5; word-wrap: break-word; }
  .msg.asst .bubble { background: var(--color-surface); color: var(--color-text); border: 1px solid var(--color-border); }
  .msg.user .bubble { background: var(--color-success); color: var(--color-bg); border-bottom-right-radius: 0.25rem; }
  .bubble.streaming { border: 1px solid rgba(62, 207, 142, 0.3); }
  .cursor { display: inline-block; width: 2px; height: 14px; background: var(--color-success); margin-left: 2px; vertical-align: text-bottom; animation: blink 1s step-end infinite; }
  @keyframes blink { 0%, 100% { opacity: 1; } 50% { opacity: 0; } }
  .typing { display: flex; align-items: center; gap: 0.5rem; padding: 0.5rem 0.75rem; background: var(--color-surface); border: 1px solid var(--color-border); border-radius: 0.75rem; width: fit-content; color: var(--color-success); }
  .dots { display: flex; gap: 4px; }
  .dots span { width: 5px; height: 5px; background: var(--color-success); border-radius: 50%; animation: bounce 1.4s infinite ease-in-out both; }
  .dots span:nth-child(1) { animation-delay: -0.32s; }
  .dots span:nth-child(2) { animation-delay: -0.16s; }
  @keyframes bounce { 0%, 80%, 100% { transform: scale(0.7); opacity: 0.4; } 40% { transform: scale(1); opacity: 1; } }
  .err { padding: 0.5rem 0.75rem; background: var(--color-error-muted); color: var(--color-error); border-radius: var(--radius-sm); font-size: 0.75rem; border: 1px solid rgba(239, 68, 68, 0.2); }
  .input { border-top: 1px solid var(--color-border); padding: 0.625rem; display: flex; align-items: flex-end; gap: 0.5rem; background: var(--color-bg-elevated); border-radius: 0 0 var(--radius-md) var(--radius-md); }
  .input textarea { flex: 1; padding: 0.5rem 0.75rem; font-size: 0.8125rem; background: var(--color-surface); border: 1px solid var(--color-border); border-radius: 1rem; color: var(--color-text); outline: none; resize: none; min-height: 36px; max-height: 80px; font-family: inherit; }
  .input textarea::placeholder { color: var(--color-text-muted); }
  .input textarea:focus { border-color: var(--color-success); box-shadow: 0 0 0 2px rgba(62,207,142,0.15); }
  .input textarea:disabled { background: var(--color-bg-elevated); opacity: 0.5; }
  .send { padding: 0.5rem; background: var(--color-success); color: var(--color-bg); border: none; border-radius: 50%; cursor: pointer; display: flex; align-items: center; justify-content: center; flex-shrink: 0; transition: all 0.2s; }
  .send:hover:not(:disabled) { background: #2eb87d; transform: scale(1.05); }
  .send:disabled { background: var(--color-surface); color: var(--color-text-muted); cursor: not-allowed; }
  :global(.spin) { animation: spin 1s linear infinite; }
  @keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
</style>
