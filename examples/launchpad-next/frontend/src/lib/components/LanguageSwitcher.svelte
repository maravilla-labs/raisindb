<script lang="ts">
  import { invalidateAll } from '$app/navigation';
  import { locale, type Locale } from '$lib/stores/locale';

  const languages: { code: Locale; label: string }[] = [
    { code: 'en', label: 'EN' },
    { code: 'de', label: 'DE' },
    { code: 'fr', label: 'FR' },
  ];

  let open = $state(false);

  function select(code: Locale) {
    locale.set(code);
    open = false;
    invalidateAll();
  }
</script>

<div class="lang-switcher">
  <button class="lang-btn" onclick={() => (open = !open)}>
    {languages.find((l) => l.code === $locale)?.label ?? 'EN'}
  </button>

  {#if open}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="lang-backdrop" onclick={() => (open = false)} onkeydown={() => {}}></div>
    <ul class="lang-menu">
      {#each languages as lang}
        <li>
          <button
            class="lang-option"
            class:active={$locale === lang.code}
            onclick={() => select(lang.code)}
          >
            {lang.label}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .lang-switcher {
    position: relative;
  }

  .lang-btn {
    padding: 0.35rem 0.6rem;
    border: 1px solid #d1d5db;
    border-radius: 6px;
    background: white;
    font-size: 0.8rem;
    font-weight: 600;
    color: #4b5563;
    cursor: pointer;
    transition: background 0.2s, color 0.2s;
  }

  .lang-btn:hover {
    background: #f3f4f6;
    color: #374151;
  }

  .lang-backdrop {
    position: fixed;
    inset: 0;
    z-index: 10;
  }

  .lang-menu {
    position: absolute;
    top: 100%;
    right: 0;
    margin-top: 0.25rem;
    background: white;
    border: 1px solid #e5e7eb;
    border-radius: 6px;
    box-shadow: 0 4px 6px -1px rgb(0 0 0 / 0.1);
    list-style: none;
    padding: 0.25rem 0;
    z-index: 20;
    min-width: 3.5rem;
  }

  .lang-option {
    width: 100%;
    padding: 0.35rem 0.75rem;
    border: none;
    background: transparent;
    font-size: 0.8rem;
    font-weight: 500;
    color: #4b5563;
    cursor: pointer;
    text-align: center;
  }

  .lang-option:hover {
    background: #f3f4f6;
  }

  .lang-option.active {
    color: #6366f1;
    font-weight: 700;
  }
</style>
