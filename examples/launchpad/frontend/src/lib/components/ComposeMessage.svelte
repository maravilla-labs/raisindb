<script lang="ts">
  import { X, Send, AlertCircle, CheckCircle } from 'lucide-svelte';
  import { sendDirectMessage, getFriends, type Friend } from '$lib/stores/messaging';
  import { onMount } from 'svelte';

  interface Props {
    onClose: () => void;
    onSent?: () => void;
    preselectedId?: string;
  }

  let { onClose, onSent, preselectedId = '' }: Props = $props();

  // Form state
  let friends = $state<Friend[]>([]);
  let selectedFriendId = $state(preselectedId);
  let subject = $state('');
  let content = $state('');
  let sending = $state(false);
  let error = $state<string | null>(null);
  let success = $state(false);
  let loading = $state(true);

  // Get selected friend from path
  const selectedFriend = $derived(friends.find(f => f.id === selectedFriendId));

  onMount(async () => {
    friends = await getFriends();
    loading = false;
  });

  async function handleSubmit(e: Event) {
    e.preventDefault();

    if (!selectedFriend) {
      error = 'Please select a friend to message';
      return;
    }

    if (!content.trim()) {
      error = 'Please enter a message';
      return;
    }

    sending = true;
    error = null;

    const result = await sendDirectMessage(
      selectedFriend.id,
      content.trim(),
      subject.trim() || undefined
    );

    if (result.success) {
      success = true;
      setTimeout(() => {
        onSent?.();
        onClose();
      }, 1500);
    } else {
      error = result.error || 'Failed to send message';
    }

    sending = false;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      onClose();
    }
  }
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="modal-overlay" onclick={onClose} role="dialog" aria-modal="true">
  <div class="modal-content" onclick={(e) => e.stopPropagation()}>
    <div class="modal-header">
      <h2>New Message</h2>
      <button class="btn-close" onclick={onClose} aria-label="Close">
        <X size={20} />
      </button>
    </div>

    {#if success}
      <div class="success-state">
        <CheckCircle size={48} />
        <h3>Message Sent!</h3>
        <p>Your message has been sent successfully.</p>
      </div>
    {:else}
      <form class="modal-body" onsubmit={handleSubmit}>
        {#if error}
          <div class="error-message">
            <AlertCircle size={16} />
            {error}
          </div>
        {/if}

        <div class="form-field">
          <label for="recipient">To</label>
          {#if loading}
            <div class="loading-placeholder">Loading friends...</div>
          {:else if friends.length === 0}
            <div class="no-friends">
              <p>You don't have any friends yet.</p>
              <a href="/friends">Add friends first</a>
            </div>
          {:else}
            <select
              id="recipient"
              bind:value={selectedFriendId}
              disabled={sending}
            >
              <option value="">Select a friend...</option>
              {#each friends as friend}
                <option value={friend.id}>
                  {friend.properties?.display_name || 'Friend'}
                </option>
              {/each}
            </select>
          {/if}
        </div>

        <div class="form-field">
          <label for="subject">Subject (optional)</label>
          <input
            type="text"
            id="subject"
            bind:value={subject}
            placeholder="What's this about?"
            disabled={sending}
          />
        </div>

        <div class="form-field">
          <label for="content">Message</label>
          <textarea
            id="content"
            bind:value={content}
            placeholder="Type your message..."
            rows="4"
            disabled={sending}
          ></textarea>
        </div>

        <div class="modal-footer">
          <button type="button" class="btn-cancel" onclick={onClose} disabled={sending}>
            Cancel
          </button>
          <button
            type="submit"
            class="btn-send"
            disabled={sending || !selectedFriend || !content.trim() || friends.length === 0}
          >
            <Send size={16} />
            {sending ? 'Sending...' : 'Send Message'}
          </button>
        </div>
      </form>
    {/if}
  </div>
</div>

<style>
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 50;
    padding: 1rem;
  }

  .modal-content {
    background: white;
    border-radius: 12px;
    width: 100%;
    max-width: 480px;
    max-height: 90vh;
    overflow: auto;
  }

  .modal-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1rem 1.5rem;
    border-bottom: 1px solid #e5e7eb;
  }

  .modal-header h2 {
    font-size: 1.125rem;
    font-weight: 600;
    margin: 0;
    color: #1f2937;
  }

  .btn-close {
    padding: 0.375rem;
    background: none;
    border: none;
    color: #6b7280;
    cursor: pointer;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .btn-close:hover {
    color: #1f2937;
    background: #f3f4f6;
  }

  .modal-body {
    padding: 1.5rem;
  }

  .success-state {
    padding: 3rem 1.5rem;
    text-align: center;
  }

  .success-state :global(svg) {
    color: #10b981;
    margin-bottom: 1rem;
  }

  .success-state h3 {
    font-size: 1.25rem;
    font-weight: 600;
    color: #1f2937;
    margin: 0 0 0.5rem;
  }

  .success-state p {
    color: #6b7280;
    margin: 0;
  }

  .error-message {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: #fef2f2;
    color: #dc2626;
    padding: 0.75rem 1rem;
    border-radius: 8px;
    font-size: 0.875rem;
    margin-bottom: 1rem;
  }

  .form-field {
    margin-bottom: 1rem;
  }

  .form-field label {
    display: block;
    font-size: 0.875rem;
    font-weight: 500;
    color: #374151;
    margin-bottom: 0.375rem;
  }

  .form-field input,
  .form-field select,
  .form-field textarea {
    width: 100%;
    padding: 0.625rem 0.875rem;
    border: 1px solid #d1d5db;
    border-radius: 8px;
    font-size: 0.875rem;
    transition: border-color 0.2s, box-shadow 0.2s;
  }

  .form-field input:focus,
  .form-field select:focus,
  .form-field textarea:focus {
    outline: none;
    border-color: #6366f1;
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.1);
  }

  .form-field textarea {
    resize: vertical;
    min-height: 100px;
  }

  .form-field input:disabled,
  .form-field select:disabled,
  .form-field textarea:disabled {
    background: #f9fafb;
    cursor: not-allowed;
  }

  .loading-placeholder {
    padding: 0.625rem 0.875rem;
    background: #f9fafb;
    border: 1px solid #e5e7eb;
    border-radius: 8px;
    color: #9ca3af;
    font-size: 0.875rem;
  }

  .no-friends {
    padding: 1rem;
    background: #fef3c7;
    border: 1px solid #fcd34d;
    border-radius: 8px;
    text-align: center;
  }

  .no-friends p {
    color: #92400e;
    margin: 0 0 0.5rem;
    font-size: 0.875rem;
  }

  .no-friends a {
    color: #6366f1;
    font-size: 0.875rem;
    font-weight: 500;
  }

  .modal-footer {
    display: flex;
    justify-content: flex-end;
    gap: 0.75rem;
    margin-top: 1.5rem;
    padding-top: 1rem;
    border-top: 1px solid #e5e7eb;
  }

  .btn-cancel {
    padding: 0.625rem 1rem;
    background: white;
    border: 1px solid #d1d5db;
    border-radius: 8px;
    color: #374151;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-cancel:hover:not(:disabled) {
    background: #f3f4f6;
  }

  .btn-send {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.625rem 1.25rem;
    background: #6366f1;
    color: white;
    border: none;
    border-radius: 8px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.2s;
  }

  .btn-send:hover:not(:disabled) {
    background: #4f46e5;
  }

  .btn-cancel:disabled,
  .btn-send:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
</style>
