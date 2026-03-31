<script lang="ts">
  import { Bot, X, Sparkles } from 'lucide-svelte';
  import { DEFAULT_AGENT, type AIAgent } from '$lib/stores/ai-chat';

  interface Props {
    agents: AIAgent[];
    onSelect: (agentPath: string) => void;
    onCancel: () => void;
  }

  let { agents, onSelect, onCancel }: Props = $props();

  // Always include default agent if not in list
  const displayAgents = $derived(() => {
    const hasDefault = agents.some(a =>
      a.path.includes(DEFAULT_AGENT.path) || a.name === 'sample-assistant'
    );
    if (!hasDefault && agents.length === 0) {
      return [{
        id: 'default',
        path: DEFAULT_AGENT.path,
        name: DEFAULT_AGENT.name,
        systemPrompt: 'A helpful AI assistant',
        model: 'gpt-4o-mini',
        provider: 'openai',
      }];
    }
    return agents;
  });

  function handleSelect(agent: AIAgent) {
    // Extract path without /functions prefix if present
    const path = agent.path.startsWith('/functions')
      ? agent.path.replace('/functions', '')
      : agent.path;
    onSelect(path);
  }

  function formatAgentName(name: string): string {
    return name
      .replace(/-/g, ' ')
      .replace(/\b\w/g, c => c.toUpperCase());
  }
</script>

<div class="agent-picker">
  <div class="picker-header">
    <Sparkles size={16} />
    <span>Choose an Agent</span>
    <button class="close-button" onclick={onCancel}>
      <X size={16} />
    </button>
  </div>

  <div class="agents-list">
    {#each displayAgents() as agent (agent.id)}
      <button class="agent-item" onclick={() => handleSelect(agent)}>
        <div class="agent-icon">
          <Bot size={20} />
        </div>
        <div class="agent-info">
          <span class="agent-name">{formatAgentName(agent.name)}</span>
          {#if agent.model}
            <span class="agent-model">{agent.model}</span>
          {/if}
        </div>
      </button>
    {/each}

    {#if displayAgents().length === 0}
      <div class="no-agents">
        <p>No agents available</p>
        <p class="hint">Agents should be defined in the functions workspace</p>
      </div>
    {/if}
  </div>

  <div class="picker-footer">
    <button class="cancel-button" onclick={onCancel}>Cancel</button>
  </div>
</div>

<style>
  .agent-picker {
    display: flex;
    flex-direction: column;
    background: #fafafa;
    min-height: 200px;
  }

  .picker-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem;
    background: white;
    border-bottom: 1px solid #e5e7eb;
    color: #6b7280;
    font-size: 0.875rem;
    font-weight: 500;
  }

  .close-button {
    margin-left: auto;
    padding: 0.25rem;
    background: transparent;
    border: none;
    border-radius: 4px;
    color: #9ca3af;
    cursor: pointer;
  }

  .close-button:hover {
    background: #f3f4f6;
    color: #6b7280;
  }

  .agents-list {
    flex: 1;
    overflow-y: auto;
    padding: 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .agent-item {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem;
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 8px;
    cursor: pointer;
    transition: all 0.2s;
    text-align: left;
  }

  .agent-item:hover {
    border-color: #8b5cf6;
    background: #faf5ff;
  }

  .agent-icon {
    width: 40px;
    height: 40px;
    background: linear-gradient(135deg, #8b5cf6, #7c3aed);
    color: white;
    border-radius: 10px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .agent-info {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
  }

  .agent-name {
    font-weight: 500;
    color: #1f2937;
    font-size: 0.875rem;
  }

  .agent-model {
    font-size: 0.75rem;
    color: #9ca3af;
  }

  .no-agents {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 2rem;
    color: #9ca3af;
    text-align: center;
  }

  .no-agents p {
    margin: 0;
  }

  .no-agents .hint {
    font-size: 0.75rem;
    margin-top: 0.5rem;
  }

  .picker-footer {
    padding: 0.5rem;
    background: white;
    border-top: 1px solid #e5e7eb;
  }

  .cancel-button {
    width: 100%;
    padding: 0.5rem;
    background: #f3f4f6;
    border: none;
    border-radius: 6px;
    color: #6b7280;
    font-size: 0.875rem;
    cursor: pointer;
    transition: background 0.2s;
  }

  .cancel-button:hover {
    background: #e5e7eb;
    color: #374151;
  }
</style>
