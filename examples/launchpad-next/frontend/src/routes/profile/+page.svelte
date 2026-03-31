<script lang="ts">
  import { user } from '$lib/stores/auth';
  import { query } from '$lib/raisin';
  import { invalidateAll } from '$app/navigation';
  import { User, Lock, Globe, Save, Plus, Trash2, AlertCircle } from 'lucide-svelte';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();

  let saving = $state(false);
  let error = $state<string | null>(data.error);
  let successMessage = $state<string | null>(null);

  let privateData = $state<Record<string, string>>({ ...(data.privateProfile?.properties?.data || {}) });
  let publicData = $state<Record<string, string>>({ ...(data.publicProfile?.properties?.data || {}) });

  let newPrivateKey = $state('');
  let newPrivateValue = $state('');
  let newPublicKey = $state('');
  let newPublicValue = $state('');

  const privateFieldSuggestions = [
    { key: 'phone', label: 'Phone Number', placeholder: '+41 61 000 0000' },
    { key: 'address', label: 'Address', placeholder: 'Steinenberg 7, 4051 Basel' },
    { key: 'dateOfBirth', label: 'Date of Birth', placeholder: '1990-01-01' },
    { key: 'emergencyContact', label: 'Emergency Contact', placeholder: 'Name: ..., Phone: +41...' },
  ];

  const publicFieldSuggestions = [
    { key: 'bio', label: 'Bio', placeholder: 'Tell us about yourself...' },
    { key: 'company', label: 'Company', placeholder: 'Acme Corp' },
    { key: 'location', label: 'Location', placeholder: 'Basel, Switzerland' },
    { key: 'website', label: 'Website', placeholder: 'https://example.com' },
    { key: 'twitter', label: 'Twitter', placeholder: '@username' },
    { key: 'github', label: 'GitHub', placeholder: 'username' },
  ];

  const USER_WORKSPACE = 'raisin:access_control';

  async function saveProfile(type: 'private' | 'public'): Promise<void> {
    const profile = type === 'private' ? data.privateProfile : data.publicProfile;
    const profileData = type === 'private' ? privateData : publicData;

    if (!profile) {
      error = `${type} profile not found.`;
      return;
    }

    try {
      saving = true;
      error = null;

      await query(`
        UPDATE "${USER_WORKSPACE}"
        SET properties = $1::jsonb
        WHERE path = $2
      `, [JSON.stringify({ data: profileData }), profile.path]);

      await invalidateAll();
      successMessage = `${type.charAt(0).toUpperCase() + type.slice(1)} profile saved!`;
      setTimeout(() => successMessage = null, 3000);
    } catch (err) {
      console.error('[profile] Save error:', err);
      error = `Failed to save ${type} profile: ${err instanceof Error ? err.message : 'Unknown error'}`;
    } finally {
      saving = false;
    }
  }

  function addField(type: 'private' | 'public') {
    const key = type === 'private' ? newPrivateKey.trim() : newPublicKey.trim();
    const value = type === 'private' ? newPrivateValue.trim() : newPublicValue.trim();
    if (!key) return;

    if (type === 'private') {
      privateData = { ...privateData, [key]: value };
      newPrivateKey = '';
      newPrivateValue = '';
    } else {
      publicData = { ...publicData, [key]: value };
      newPublicKey = '';
      newPublicValue = '';
    }
  }

  function removeField(type: 'private' | 'public', key: string) {
    if (type === 'private') {
      const { [key]: _, ...rest } = privateData;
      privateData = rest;
    } else {
      const { [key]: _, ...rest } = publicData;
      publicData = rest;
    }
  }

  function updateField(type: 'private' | 'public', key: string, value: string) {
    if (type === 'private') {
      privateData = { ...privateData, [key]: value };
    } else {
      publicData = { ...publicData, [key]: value };
    }
  }

  function addSuggestedField(type: 'private' | 'public', key: string) {
    if (type === 'private') {
      if (!(key in privateData)) privateData = { ...privateData, [key]: '' };
    } else {
      if (!(key in publicData)) publicData = { ...publicData, [key]: '' };
    }
  }
</script>

<svelte:head>
  <title>Profile - Nachtkultur</title>
</svelte:head>

<div class="profile-page">
  <div class="profile-header">
    <div class="header-content">
      <div class="header-icon">
        <User size={20} />
      </div>
      <div>
        <h1>My Profile</h1>
        {#if $user}
          <p class="user-email">{$user.email}</p>
        {/if}
      </div>
    </div>
  </div>

  {#if error}
    <div class="error-message">
      <AlertCircle size={16} />
      {error}
    </div>
  {:else}
    {#if successMessage}
      <div class="success-message">{successMessage}</div>
    {/if}

    <div class="profile-sections">
      <!-- Private Section -->
      <section class="profile-section">
        <div class="section-header private">
          <div class="section-title">
            <Lock size={16} />
            <h2>Private</h2>
          </div>
          <p class="section-description">Only you can see this</p>
        </div>

        <div class="fields-container">
          {#each Object.entries(privateData) as [key, value]}
            <div class="field-row">
              <label class="field-label">{key}</label>
              <input
                type="text"
                class="field-input"
                value={value}
                oninput={(e) => updateField('private', key, e.currentTarget.value)}
                placeholder={privateFieldSuggestions.find(f => f.key === key)?.placeholder || ''}
              />
              <button class="btn-remove" onclick={() => removeField('private', key)} title="Remove">
                <Trash2 size={14} />
              </button>
            </div>
          {/each}

          <div class="add-field-row">
            <input type="text" class="field-key-input" placeholder="Field name" bind:value={newPrivateKey} onkeydown={(e) => e.key === 'Enter' && addField('private')} />
            <input type="text" class="field-value-input" placeholder="Value" bind:value={newPrivateValue} onkeydown={(e) => e.key === 'Enter' && addField('private')} />
            <button class="btn-add" onclick={() => addField('private')} disabled={!newPrivateKey.trim()}>
              <Plus size={14} />
            </button>
          </div>

          <div class="suggested-fields">
            <span class="suggested-label">Add:</span>
            {#each privateFieldSuggestions.filter(f => !(f.key in privateData)) as suggestion}
              <button class="suggestion-chip" onclick={() => addSuggestedField('private', suggestion.key)}>
                + {suggestion.label}
              </button>
            {/each}
          </div>

          <button class="btn-save" onclick={() => saveProfile('private')} disabled={saving}>
            <Save size={14} />
            {saving ? 'Saving...' : 'Save Private Profile'}
          </button>
        </div>
      </section>

      <!-- Public Section -->
      <section class="profile-section">
        <div class="section-header public">
          <div class="section-title">
            <Globe size={16} />
            <h2>Public</h2>
          </div>
          <p class="section-description">Visible to others</p>
        </div>

        <div class="fields-container">
          {#each Object.entries(publicData) as [key, value]}
            <div class="field-row">
              <label class="field-label">{key}</label>
              <input
                type="text"
                class="field-input"
                value={value}
                oninput={(e) => updateField('public', key, e.currentTarget.value)}
                placeholder={publicFieldSuggestions.find(f => f.key === key)?.placeholder || ''}
              />
              <button class="btn-remove" onclick={() => removeField('public', key)} title="Remove">
                <Trash2 size={14} />
              </button>
            </div>
          {/each}

          <div class="add-field-row">
            <input type="text" class="field-key-input" placeholder="Field name" bind:value={newPublicKey} onkeydown={(e) => e.key === 'Enter' && addField('public')} />
            <input type="text" class="field-value-input" placeholder="Value" bind:value={newPublicValue} onkeydown={(e) => e.key === 'Enter' && addField('public')} />
            <button class="btn-add" onclick={() => addField('public')} disabled={!newPublicKey.trim()}>
              <Plus size={14} />
            </button>
          </div>

          <div class="suggested-fields">
            <span class="suggested-label">Add:</span>
            {#each publicFieldSuggestions.filter(f => !(f.key in publicData)) as suggestion}
              <button class="suggestion-chip" onclick={() => addSuggestedField('public', suggestion.key)}>
                + {suggestion.label}
              </button>
            {/each}
          </div>

          <button class="btn-save" onclick={() => saveProfile('public')} disabled={saving}>
            <Save size={14} />
            {saving ? 'Saving...' : 'Save Public Profile'}
          </button>
        </div>
      </section>
    </div>
  {/if}
</div>

<style>
  .profile-page {
    max-width: 1000px;
    margin: 0 auto;
    padding: 2.5rem 2rem;
    animation: fadeInUp 0.4s ease both;
  }

  .profile-header {
    margin-bottom: 2rem;
  }

  .header-content {
    display: flex;
    align-items: flex-start;
    gap: 1rem;
  }

  .header-icon {
    width: 40px;
    height: 40px;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--color-accent);
    flex-shrink: 0;
  }

  .profile-header h1 {
    font-family: var(--font-display);
    font-size: 1.5rem;
    font-weight: 600;
    color: var(--color-text-heading);
    margin: 0;
    letter-spacing: -0.02em;
  }

  .user-email {
    color: var(--color-text-muted);
    margin: 0.25rem 0 0;
    font-size: 0.85rem;
  }

  .error-message {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: var(--color-error-muted);
    color: var(--color-error);
    padding: 1rem;
    border-radius: var(--radius-md);
    margin-bottom: 1rem;
    border: 1px solid rgba(239, 68, 68, 0.15);
    font-size: 0.85rem;
  }

  .success-message {
    background: var(--color-success-muted);
    color: var(--color-success);
    padding: 0.75rem 1rem;
    border-radius: var(--radius-md);
    margin-bottom: 1rem;
    text-align: center;
    font-size: 0.85rem;
    border: 1px solid rgba(62, 207, 142, 0.15);
  }

  .profile-sections {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(400px, 1fr));
    gap: 1.5rem;
  }

  .profile-section {
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-lg);
    overflow: hidden;
  }

  .section-header {
    padding: 1.25rem 1.5rem;
    border-bottom: 1px solid var(--color-border-subtle);
  }

  .section-header.private {
    border-left: 3px solid var(--color-warning);
  }

  .section-header.public {
    border-left: 3px solid var(--color-accent);
  }

  .section-title {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .section-title h2 {
    font-family: var(--font-display);
    font-size: 1rem;
    font-weight: 600;
    color: var(--color-text-heading);
    margin: 0;
  }

  .section-header.private .section-title :global(svg) {
    color: var(--color-warning);
  }

  .section-header.public .section-title :global(svg) {
    color: var(--color-accent);
  }

  .section-description {
    font-size: 0.8rem;
    color: var(--color-text-muted);
    margin: 0.375rem 0 0;
  }

  .fields-container {
    padding: 1.5rem;
  }

  .field-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 0.75rem;
  }

  .field-label {
    min-width: 110px;
    font-weight: 500;
    color: var(--color-text-secondary);
    font-size: 0.8rem;
    letter-spacing: 0.01em;
  }

  .field-input {
    flex: 1;
    padding: 0.5rem 0.75rem;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font-size: 0.85rem;
    color: var(--color-text);
    font-family: var(--font-body);
    transition: border-color 0.2s;
  }

  .field-input:focus {
    outline: none;
    border-color: var(--color-accent);
  }

  .btn-remove {
    padding: 0.4rem;
    background: transparent;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-remove:hover {
    background: var(--color-error-muted);
    border-color: rgba(239, 68, 68, 0.3);
    color: var(--color-error);
  }

  .add-field-row {
    display: flex;
    gap: 0.5rem;
    margin: 1.25rem 0 0;
    padding-top: 1.25rem;
    border-top: 1px solid var(--color-border-subtle);
  }

  .field-key-input {
    width: 110px;
    padding: 0.5rem 0.75rem;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font-size: 0.85rem;
    color: var(--color-text);
    font-family: var(--font-body);
  }

  .field-value-input {
    flex: 1;
    padding: 0.5rem 0.75rem;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    font-size: 0.85rem;
    color: var(--color-text);
    font-family: var(--font-body);
  }

  .field-key-input:focus, .field-value-input:focus {
    outline: none;
    border-color: var(--color-accent);
  }

  .field-key-input::placeholder, .field-value-input::placeholder, .field-input::placeholder {
    color: var(--color-text-muted);
  }

  .btn-add {
    padding: 0.4rem 0.5rem;
    background: var(--color-success-muted);
    border: 1px solid rgba(62, 207, 142, 0.2);
    border-radius: var(--radius-sm);
    color: var(--color-success);
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-add:hover:not(:disabled) {
    background: rgba(62, 207, 142, 0.2);
  }

  .btn-add:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .suggested-fields {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem;
    margin: 1rem 0;
  }

  .suggested-label {
    font-size: 0.7rem;
    color: var(--color-text-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .suggestion-chip {
    padding: 0.2rem 0.6rem;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: 999px;
    font-size: 0.7rem;
    color: var(--color-text-secondary);
    cursor: pointer;
    transition: all 0.2s;
    font-family: var(--font-body);
  }

  .suggestion-chip:hover {
    border-color: var(--color-accent);
    color: var(--color-accent);
    background: var(--color-accent-glow);
  }

  .btn-save {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.7rem;
    background: var(--color-accent);
    color: var(--color-bg);
    border: none;
    border-radius: var(--radius-md);
    font-weight: 600;
    font-size: 0.8rem;
    cursor: pointer;
    transition: all 0.2s;
    margin-top: 1rem;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    font-family: var(--font-body);
  }

  .btn-save:hover:not(:disabled) {
    background: var(--color-accent-hover);
    box-shadow: 0 4px 16px rgba(212, 175, 55, 0.2);
  }

  .btn-save:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  @media (max-width: 768px) {
    .profile-page {
      padding: 1.5rem 1rem;
    }

    .profile-sections {
      grid-template-columns: 1fr;
    }

    .field-row {
      flex-wrap: wrap;
    }

    .field-label {
      min-width: 100%;
      margin-bottom: 0.25rem;
    }

    .field-input {
      flex: 1;
      min-width: 0;
    }
  }
</style>
