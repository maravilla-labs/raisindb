import { writable } from 'svelte/store';
import { browser } from '$app/environment';

export type Locale = 'en' | 'de' | 'fr';

const STORAGE_KEY = 'launchpad-locale';

function getInitialLocale(): Locale {
  if (browser) {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === 'de' || stored === 'fr') return stored;
  }
  return 'en';
}

export const locale = writable<Locale>(getInitialLocale());

// Persist to localStorage on change
if (browser) {
  locale.subscribe((value) => {
    localStorage.setItem(STORAGE_KEY, value);
  });
}

/**
 * Sync read of current locale (for use in load functions which aren't reactive)
 */
export function getCurrentLocale(): Locale {
  if (browser) {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === 'de' || stored === 'fr') return stored;
  }
  return 'en';
}

/**
 * Returns a SQL AND clause for the current locale.
 * English (default) returns empty string — no filtering needed.
 */
export function localeClause(): string {
  const current = getCurrentLocale();
  if (current === 'en') return '';
  return `AND locale = '${current}'`;
}
