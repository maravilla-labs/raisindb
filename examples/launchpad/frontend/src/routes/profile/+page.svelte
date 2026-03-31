<script lang="ts">
  import { user } from '$lib/stores/auth';
  import { query } from '$lib/raisin';
  import { invalidateAll } from '$app/navigation';
  import { User, Lock, Globe, Save, Plus, Trash2, AlertCircle } from 'lucide-svelte';
  import type { PageData } from './$types';

  // Props from load function
  let { data }: { data: PageData } = $props();

  // State
  let saving = $state(false);
  let error = $state<string | null>(data.error);
  let successMessage = $state<string | null>(null);

  // Editable data (initialized from load data)
  let privateData = $state<Record<string, string>>({ ...(data.privateProfile?.properties?.data || {}) });
  let publicData = $state<Record<string, string>>({ ...(data.publicProfile?.properties?.data || {}) });

  // New field inputs
  let newPrivateKey = $state('');
  let newPrivateValue = $state('');
  let newPublicKey = $state('');
  let newPublicValue = $state('');

  // Suggested fields for private profile
  const privateFieldSuggestions = [
    { key: 'phone', label: 'Phone Number', placeholder: '+1 234 567 8900' },
    { key: 'address', label: 'Address', placeholder: '123 Main St, City, Country' },
    { key: 'dateOfBirth', label: 'Date of Birth', placeholder: '1990-01-01' },
    { key: 'emergencyContact', label: 'Emergency Contact', placeholder: 'Name: John, Phone: +1...' },
  ];

  // Suggested fields for public profile
  const publicFieldSuggestions = [
    { key: 'bio', label: 'Bio', placeholder: 'Tell us about yourself...' },
    { key: 'company', label: 'Company', placeholder: 'Acme Corp' },
    { key: 'location', label: 'Location', placeholder: 'San Francisco, CA' },
    { key: 'website', label: 'Website', placeholder: 'https://example.com' },
    { key: 'twitter', label: 'Twitter', placeholder: '@username' },
    { key: 'github', label: 'GitHub', placeholder: 'username' },
  ];

  // The user home is always in the raisin:access_control workspace
  const USER_WORKSPACE = 'raisin:access_control';

  // Save profile data
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

      // Reload page data
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

  // Add new field
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

  // Remove field
  function removeField(type: 'private' | 'public', key: string) {
    if (type === 'private') {
      const { [key]: _, ...rest } = privateData;
      privateData = rest;
    } else {
      const { [key]: _, ...rest } = publicData;
      publicData = rest;
    }
  }

  // Update field value
  function updateField(type: 'private' | 'public', key: string, value: string) {
    if (type === 'private') {
      privateData = { ...privateData, [key]: value };
    } else {
      publicData = { ...publicData, [key]: value };
    }
  }

  // Add suggested field
  function addSuggestedField(type: 'private' | 'public', key: string) {
    if (type === 'private') {
      if (!(key in privateData)) {
        privateData = { ...privateData, [key]: '' };
      }
    } else {
      if (!(key in publicData)) {
        publicData = { ...publicData, [key]: '' };
      }
    }
  }
</script>

<svelte:head>
  <title>Profile - Launchpad</title>
</svelte:head>

<div class="profile-page">
  <div class="profile-header">
    <div class="header-content">
      <User size={32} />
      <div>
        <h1>My Profile</h1>
        {#if $user}
          <p class="user-email">{$user.email}</p>
          <p class="user-home">{$user.home || 'No home path'}</p>
        {/if}
      </div>
    </div>
  </div>

  {#if error}
    <div class="error-message">
      <AlertCircle size={20} />
      {error}
    </div>
  {:else}
    {#if successMessage}
      <div class="success-message">{successMessage}</div>
    {/if}

    <div class="profile-sections">
      <!-- Private Section -->
      <section class="profile-section private-section">
        <div class="section-header">
          <div class="section-title">
            <Lock size={20} />
            <h2>Private Information</h2>
          </div>
          <p class="section-description">Only you can see this information</p>
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
                <button class="btn-remove" onclick={() => removeField('private', key)} title="Remove field">
                  <Trash2 size={16} />
                </button>
              </div>
            {/each}

            <!-- Add new field -->
            <div class="add-field-row">
              <input
                type="text"
                class="field-key-input"
                placeholder="Field name"
                bind:value={newPrivateKey}
                onkeydown={(e) => e.key === 'Enter' && addField('private')}
              />
              <input
                type="text"
                class="field-value-input"
                placeholder="Value"
                bind:value={newPrivateValue}
                onkeydown={(e) => e.key === 'Enter' && addField('private')}
              />
              <button class="btn-add" onclick={() => addField('private')} disabled={!newPrivateKey.trim()}>
                <Plus size={16} />
              </button>
            </div>

            <!-- Suggested fields -->
            <div class="suggested-fields">
              <span class="suggested-label">Suggestions:</span>
              {#each privateFieldSuggestions.filter(f => !(f.key in privateData)) as suggestion}
                <button class="suggestion-chip" onclick={() => addSuggestedField('private', suggestion.key)}>
                  + {suggestion.label}
                </button>
              {/each}
            </div>

            <button class="btn-save" onclick={() => saveProfile('private')} disabled={saving}>
              <Save size={16} />
              {saving ? 'Saving...' : 'Save Private Profile'}
            </button>
        </div>
      </section>

      <!-- Public Section -->
      <section class="profile-section public-section">
        <div class="section-header">
          <div class="section-title">
            <Globe size={20} />
            <h2>Public Information</h2>
          </div>
          <p class="section-description">This information may be visible to others</p>
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
                <button class="btn-remove" onclick={() => removeField('public', key)} title="Remove field">
                  <Trash2 size={16} />
                </button>
              </div>
            {/each}

            <!-- Add new field -->
            <div class="add-field-row">
              <input
                type="text"
                class="field-key-input"
                placeholder="Field name"
                bind:value={newPublicKey}
                onkeydown={(e) => e.key === 'Enter' && addField('public')}
              />
              <input
                type="text"
                class="field-value-input"
                placeholder="Value"
                bind:value={newPublicValue}
                onkeydown={(e) => e.key === 'Enter' && addField('public')}
              />
              <button class="btn-add" onclick={() => addField('public')} disabled={!newPublicKey.trim()}>
                <Plus size={16} />
              </button>
            </div>

            <!-- Suggested fields -->
            <div class="suggested-fields">
              <span class="suggested-label">Suggestions:</span>
              {#each publicFieldSuggestions.filter(f => !(f.key in publicData)) as suggestion}
                <button class="suggestion-chip" onclick={() => addSuggestedField('public', suggestion.key)}>
                  + {suggestion.label}
                </button>
              {/each}
            </div>

            <button class="btn-save" onclick={() => saveProfile('public')} disabled={saving}>
              <Save size={16} />
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
    padding: 2rem;
  }

  .profile-header {
    margin-bottom: 2rem;
  }

  .header-content {
    display: flex;
    align-items: flex-start;
    gap: 1rem;
  }

  .header-content :global(svg) {
    color: #6366f1;
    flex-shrink: 0;
    margin-top: 0.25rem;
  }

  .profile-header h1 {
    font-size: 1.75rem;
    font-weight: 700;
    color: #1f2937;
    margin: 0;
  }

  .user-email {
    color: #4b5563;
    margin: 0.25rem 0 0;
  }

  .user-home {
    font-family: ui-monospace, monospace;
    font-size: 0.875rem;
    color: #9ca3af;
    margin: 0.25rem 0 0;
  }

  .error-message {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: #fef2f2;
    color: #dc2626;
    padding: 1rem;
    border-radius: 8px;
    margin-bottom: 1rem;
  }

  .success-message {
    background: #f0fdf4;
    color: #16a34a;
    padding: 1rem;
    border-radius: 8px;
    margin-bottom: 1rem;
    text-align: center;
  }

  .profile-sections {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(400px, 1fr));
    gap: 2rem;
  }

  .profile-section {
    background: white;
    border-radius: 12px;
    border: 1px solid #e5e7eb;
    overflow: hidden;
  }

  .section-header {
    padding: 1.25rem 1.5rem;
    border-bottom: 1px solid #e5e7eb;
  }

  .private-section .section-header {
    background: linear-gradient(135deg, #fef3c7 0%, #fde68a 100%);
  }

  .public-section .section-header {
    background: linear-gradient(135deg, #dbeafe 0%, #bfdbfe 100%);
  }

  .section-title {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .section-title h2 {
    font-size: 1.125rem;
    font-weight: 600;
    color: #1f2937;
    margin: 0;
  }

  .private-section .section-title :global(svg) {
    color: #d97706;
  }

  .public-section .section-title :global(svg) {
    color: #2563eb;
  }

  .section-description {
    font-size: 0.875rem;
    color: #6b7280;
    margin: 0.5rem 0 0;
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
    min-width: 120px;
    font-weight: 500;
    color: #374151;
    font-size: 0.875rem;
  }

  .field-input {
    flex: 1;
    padding: 0.5rem 0.75rem;
    border: 1px solid #d1d5db;
    border-radius: 6px;
    font-size: 0.875rem;
  }

  .field-input:focus {
    outline: none;
    border-color: #6366f1;
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.1);
  }

  .btn-remove {
    padding: 0.5rem;
    background: transparent;
    border: 1px solid #e5e7eb;
    border-radius: 6px;
    color: #9ca3af;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-remove:hover {
    background: #fef2f2;
    border-color: #fecaca;
    color: #dc2626;
  }

  .add-field-row {
    display: flex;
    gap: 0.5rem;
    margin: 1rem 0;
    padding-top: 1rem;
    border-top: 1px dashed #e5e7eb;
  }

  .field-key-input {
    width: 120px;
    padding: 0.5rem 0.75rem;
    border: 1px solid #d1d5db;
    border-radius: 6px;
    font-size: 0.875rem;
  }

  .field-value-input {
    flex: 1;
    padding: 0.5rem 0.75rem;
    border: 1px solid #d1d5db;
    border-radius: 6px;
    font-size: 0.875rem;
  }

  .field-key-input:focus,
  .field-value-input:focus {
    outline: none;
    border-color: #6366f1;
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.1);
  }

  .btn-add {
    padding: 0.5rem;
    background: #f0fdf4;
    border: 1px solid #86efac;
    border-radius: 6px;
    color: #16a34a;
    cursor: pointer;
    transition: all 0.2s;
  }

  .btn-add:hover:not(:disabled) {
    background: #dcfce7;
  }

  .btn-add:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .suggested-fields {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    margin: 1rem 0;
  }

  .suggested-label {
    font-size: 0.75rem;
    color: #9ca3af;
  }

  .suggestion-chip {
    padding: 0.25rem 0.75rem;
    background: #f3f4f6;
    border: 1px solid #e5e7eb;
    border-radius: 999px;
    font-size: 0.75rem;
    color: #6b7280;
    cursor: pointer;
    transition: all 0.2s;
  }

  .suggestion-chip:hover {
    background: #e5e7eb;
    color: #374151;
  }

  .btn-save {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.75rem;
    background: #6366f1;
    color: white;
    border: none;
    border-radius: 8px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.2s;
    margin-top: 1rem;
  }

  .btn-save:hover:not(:disabled) {
    background: #4f46e5;
  }

  .btn-save:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  @media (max-width: 768px) {
    .profile-page {
      padding: 1rem;
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
