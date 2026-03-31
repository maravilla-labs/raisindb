<script lang="ts">
  import { Bot, User, ChevronDown, ChevronRight, Lightbulb, Wrench } from 'lucide-svelte';
  import { marked } from 'marked';
  import type { AIMessage, AIMessageChild } from '$lib/stores/ai-chat';

  interface Props {
    message: AIMessage;
  }

  let { message }: Props = $props();

  // Configure marked for safe rendering
  marked.setOptions({
    breaks: true,  // Convert \n to <br>
    gfm: true,     // GitHub Flavored Markdown
  });

  // Render markdown content to HTML
  function renderMarkdown(content: string): string {
    if (!content) return '';
    try {
      return marked.parse(content) as string;
    } catch {
      return content;
    }
  }

  const isUser = $derived(message.role === 'user');
  const isAssistant = $derived(message.role === 'assistant');

  // Check if this message has tool calls or thoughts (for showing placeholder text)
  const hasToolCalls = $derived(message.children?.some(c => c.type === 'tool_call') ?? false);
  const hasThoughts = $derived(message.children?.some(c => c.type === 'thought') ?? false);

  // Should we show the bubble? Only if there's content, or it's optimistic, or has tool calls/thoughts
  const showBubble = $derived(!!message.content || message._optimistic || hasToolCalls || hasThoughts);

  // Track expanded state for children
  let expandedChildren = $state<Set<string>>(new Set());

  function toggleChild(childId: string) {
    if (expandedChildren.has(childId)) {
      expandedChildren = new Set([...expandedChildren].filter(id => id !== childId));
    } else {
      expandedChildren = new Set([...expandedChildren, childId]);
    }
  }

  function formatTime(timestamp: string): string {
    try {
      return new Date(timestamp).toLocaleTimeString([], {
        hour: '2-digit',
        minute: '2-digit'
      });
    } catch {
      return '';
    }
  }
</script>

<div class="message-wrapper" class:user={isUser} class:assistant={isAssistant}>
  <!-- Avatar -->
  <div class="avatar" class:user-avatar={isUser} class:assistant-avatar={isAssistant}>
    {#if isUser}
      <User size={14} />
    {:else}
      <Bot size={14} />
    {/if}
  </div>

  <!-- Content -->
  <div class="message-content">
    {#if showBubble}
      <div class="bubble" class:user-bubble={isUser} class:assistant-bubble={isAssistant}>
        {#if message.content}
          <div class="message-text markdown-content">
            {@html renderMarkdown(message.content)}
          </div>
        {:else if message._optimistic}
          <p class="message-text sending">Sending...</p>
        {:else if hasToolCalls}
          <p class="message-text using-tools">Using tools...</p>
        {:else if hasThoughts}
          <p class="message-text thinking">Thinking...</p>
        {/if}
      </div>
    {/if}

    <!-- Children (thoughts, tool calls) -->
    {#if message.children && message.children.length > 0}
      <div class="children-container">
        {#each message.children as child (child.id)}
          <div class="child-item">
            <button class="child-header" onclick={() => toggleChild(child.id)}>
              {#if expandedChildren.has(child.id)}
                <ChevronDown size={12} />
              {:else}
                <ChevronRight size={12} />
              {/if}

              {#if child.type === 'thought'}
                <span class="thought-icon"><Lightbulb size={12} /></span>
                <span>Thinking</span>
              {:else if child.type === 'tool_call'}
                <span class="tool-icon {child.status || ''}"><Wrench size={12} /></span>
                <span>Tool: {child.toolName || 'unknown'}</span>
                {#if child.status && child.status !== 'completed'}
                  <span class="status-badge {child.status || ''}">
                    {child.status}
                  </span>
                {/if}
              {:else if child.type === 'tool_result'}
                <span class="result-icon"><Wrench size={12} /></span>
                <span>Result: {child.toolName || ''}</span>
              {/if}
            </button>

            {#if expandedChildren.has(child.id)}
              <div class="child-content">
                <pre>{child.type === 'tool_call' && child.toolInput
                  ? JSON.stringify(child.toolInput, null, 2)
                  : child.content}</pre>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}

    <!-- Timestamp -->
    {#if message.timestamp}
      <span class="timestamp">{formatTime(message.timestamp)}</span>
    {/if}
  </div>
</div>

<style>
  .message-wrapper {
    display: flex;
    gap: 0.5rem;
    max-width: 100%;
  }

  .message-wrapper.user {
    flex-direction: row-reverse;
  }

  .avatar {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }

  .user-avatar {
    background: #dbeafe;
    color: #3b82f6;
  }

  .assistant-avatar {
    background: #ede9fe;
    color: #8b5cf6;
  }

  .message-content {
    display: flex;
    flex-direction: column;
    max-width: 85%;
  }

  .message-wrapper.user .message-content {
    align-items: flex-end;
  }

  .bubble {
    padding: 0.5rem 0.75rem;
    border-radius: 12px;
    font-size: 0.875rem;
    line-height: 1.4;
    word-break: break-word;
  }

  .user-bubble {
    background: linear-gradient(135deg, #3b82f6, #2563eb);
    color: white;
    border-bottom-right-radius: 4px;
  }

  .assistant-bubble {
    background: white;
    color: #1f2937;
    border: 1px solid #e5e7eb;
    border-bottom-left-radius: 4px;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }

  .message-text {
    margin: 0;
    white-space: pre-wrap;
  }

  /* Markdown content styling */
  .markdown-content {
    white-space: normal;
  }

  .markdown-content :global(p) {
    margin: 0 0 0.5rem 0;
  }

  .markdown-content :global(p:last-child) {
    margin-bottom: 0;
  }

  .markdown-content :global(ul),
  .markdown-content :global(ol) {
    margin: 0.25rem 0;
    padding-left: 1.25rem;
  }

  .markdown-content :global(li) {
    margin: 0.125rem 0;
  }

  .markdown-content :global(strong) {
    font-weight: 600;
  }

  .markdown-content :global(em) {
    font-style: italic;
  }

  .markdown-content :global(code) {
    background: rgba(0, 0, 0, 0.1);
    padding: 0.125rem 0.25rem;
    border-radius: 3px;
    font-family: 'SF Mono', Menlo, Monaco, monospace;
    font-size: 0.8125rem;
  }

  .markdown-content :global(pre) {
    background: #1f2937;
    color: #d1d5db;
    padding: 0.5rem;
    border-radius: 6px;
    overflow-x: auto;
    margin: 0.5rem 0;
  }

  .markdown-content :global(pre code) {
    background: transparent;
    padding: 0;
  }

  .markdown-content :global(blockquote) {
    border-left: 3px solid #d1d5db;
    margin: 0.5rem 0;
    padding-left: 0.75rem;
    color: #6b7280;
  }

  .markdown-content :global(a) {
    color: #3b82f6;
    text-decoration: underline;
  }

  .markdown-content :global(h1),
  .markdown-content :global(h2),
  .markdown-content :global(h3) {
    margin: 0.5rem 0 0.25rem 0;
    font-weight: 600;
  }

  .markdown-content :global(h1) { font-size: 1.125rem; }
  .markdown-content :global(h2) { font-size: 1rem; }
  .markdown-content :global(h3) { font-size: 0.9375rem; }

  .message-text.sending {
    color: rgba(255, 255, 255, 0.7);
    font-style: italic;
  }

  .message-text.using-tools,
  .message-text.thinking {
    color: #9ca3af;
    font-style: italic;
  }

  .children-container {
    margin-top: 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .child-item {
    background: #f9fafb;
    border: 1px solid #e5e7eb;
    border-radius: 8px;
    overflow: hidden;
  }

  .child-header {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.375rem 0.5rem;
    font-size: 0.75rem;
    color: #6b7280;
    background: transparent;
    border: none;
    cursor: pointer;
    width: 100%;
    text-align: left;
  }

  .child-header:hover {
    background: #f3f4f6;
  }

  .thought-icon {
    color: #eab308;
  }

  .tool-icon {
    color: #f59e0b;
  }

  .tool-icon.pending {
    color: #eab308;
  }

  .tool-icon.running {
    color: #3b82f6;
    animation: pulse 1s infinite;
  }

  .tool-icon.completed {
    color: #22c55e;
  }

  .result-icon {
    color: #22c55e;
  }

  .status-badge {
    font-size: 0.625rem;
    padding: 0.125rem 0.375rem;
    border-radius: 999px;
    font-weight: 500;
  }

  .status-badge.pending {
    background: #fef3c7;
    color: #92400e;
  }

  .status-badge.running {
    background: #dbeafe;
    color: #1e40af;
  }

  .child-content {
    padding: 0.5rem;
    background: #1f2937;
    border-top: 1px solid #e5e7eb;
  }

  .child-content pre {
    margin: 0;
    font-size: 0.75rem;
    color: #d1d5db;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: 'SF Mono', Menlo, Monaco, monospace;
  }

  .timestamp {
    font-size: 0.625rem;
    color: #9ca3af;
    margin-top: 0.25rem;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }
</style>
