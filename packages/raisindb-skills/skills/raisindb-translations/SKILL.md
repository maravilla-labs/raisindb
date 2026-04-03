---
name: raisindb-translations
description: "Multi-language content with translation files and locale-based queries. Use when adding internationalization to your RaisinDB app."
---

# RaisinDB Translations

## How Translations Work

RaisinDB uses a file-overlay system for multi-language content:

- **Base content** lives in `.node.yaml` and serves as the default language (typically English).
- **Translation overlays** live in `.node.{locale}.yaml` files alongside the base file (e.g., `.node.fr.yaml`, `.node.de.yaml`).
- Only fields marked `translatable: true` in their archetype or element type definition appear in translation files.
- At query time, the server merges the base content with the requested locale's overlay.

```
content/launchpad/home/
  .node.yaml        # Base (English)
  .node.fr.yaml     # French overlay
  .node.de.yaml     # German overlay
```

## Mark Fields as Translatable

Add `translatable: true` to fields that need translation in archetypes and element types. Fields without this flag must not appear in translation files.

**Archetype** (`archetypes/landing-page.yaml`):

```yaml
fields:
  - $type: TextField
    name: title
    required: true
    translatable: true          # Translated
  - $type: TextField
    name: slug
    required: true              # NOT translated — same across locales
  - $type: TextField
    name: description
    translatable: true          # Translated
  - $type: SectionField
    name: content
    allowed_element_types: [launchpad:Hero, launchpad:TextBlock]
```

**Element type** (`elementtypes/hero.yaml`):

```yaml
fields:
  - { $type: TextField, name: headline, translatable: true }
  - { $type: TextField, name: subheadline, translatable: true }
  - { $type: TextField, name: cta_text, translatable: true }
  - { $type: TextField, name: cta_link }  # NOT translated
```

## Translation File Format

Translation files contain **only** translated properties. They omit `node_type`, `archetype`, and non-translatable fields. Section elements are matched by `uuid`.

**Base file** (`.node.yaml`):

```yaml
node_type: launchpad:Page
archetype: launchpad:LandingPage
properties:
  title: Welcome to Launchpad
  slug: home
  description: Your gateway to launching amazing projects
  content:
    - uuid: hero-1
      element_type: launchpad:Hero
      headline: Launch Your Vision
      subheadline: Build, deploy, and scale your ideas with Launchpad
      cta_text: Get Started
      cta_link: /contact
    - uuid: intro-1
      element_type: launchpad:TextBlock
      heading: Why Launchpad?
      content: |
        Launchpad is your all-in-one platform for turning ideas into reality.
```

**French** (`.node.fr.yaml`):

```yaml
title: Bienvenue sur Launchpad
description: Votre passerelle pour lancer des projets exceptionnels
content:
  - uuid: hero-1
    headline: Lancez votre vision
    subheadline: Construisez, deployez et faites evoluer vos idees avec Launchpad
    cta_text: Commencer
  - uuid: intro-1
    heading: Pourquoi Launchpad ?
    content: |
      Launchpad est votre plateforme tout-en-un pour concretiser vos idees.
```

**German** (`.node.de.yaml`):

```yaml
title: Willkommen bei Launchpad
description: Ihr Tor zum Start grossartiger Projekte
content:
  - uuid: hero-1
    headline: Starten Sie Ihre Vision
    subheadline: Bauen, deployen und skalieren Sie Ihre Ideen mit Launchpad
    cta_text: Jetzt starten
  - uuid: intro-1
    heading: Warum Launchpad?
    content: |
      Launchpad ist Ihre All-in-One-Plattform, um Ideen in die Realitat umzusetzen.
```

Key rules:

- No `node_type` or `archetype` -- those belong only in the base file.
- No non-translatable fields (`slug`, `cta_link`, etc.).
- `uuid` must match the base file exactly; `element_type` can be omitted.

## Frontend Locale Store

Track the active language and generate SQL clauses. The key function is `localeClause()`:

```typescript
// lib/stores/locale.ts
export type Locale = 'en' | 'de' | 'fr';

export const locale = writable<Locale>(getInitialLocale());

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
```

The default locale (English) returns an empty string so the base content is used without filtering.

## Querying with Locale

Append `localeClause()` to any SQL query that returns translatable content:

```typescript
import { localeClause } from '$lib/stores/locale';

export async function getPageByPath(path: string): Promise<PageNode | null> {
  const sql = `
    SELECT id, path, name, node_type, archetype, properties
    FROM ${WORKSPACE_NAME}
    WHERE path = $1 ${localeClause()}
    LIMIT 1
  `;
  return queryOne<PageNode>(sql, [nodePath]);
}

export async function getNavigation(): Promise<NavItem[]> {
  const sql = `
    SELECT id, path, name, node_type, properties
    FROM ${WORKSPACE_NAME}
    WHERE CHILD_OF('/${WORKSPACE_NAME}')
      AND node_type = 'launchpad:Page'
      ${localeClause()}
  `;
  return query<NavItem>(sql);
}
```

The server merges the locale overlay onto the base content before returning results.

## Supported Locales

RaisinDB uses BCP 47 language codes: `en`, `fr`, `de`, `es`, `pt-BR`, `zh-Hans`, `ja`, `ko`, `ar`, `it`, and any valid BCP 47 code. Add a new locale by creating `.node.{locale}.yaml` files alongside your base content.

## Validation

**MANDATORY** — run after every translation file change:

    npm run validate

Common errors:

| Error | Cause | Fix |
|-------|-------|-----|
| `TRANSLATION_FIELD_NOT_TRANSLATABLE` | Translation includes a non-translatable field | Remove the field or add `translatable: true` to the type definition |
| `TRANSLATION_MISSING_UUID` | Element uuid has no match in the base file | Ensure uuid matches an element in `.node.yaml` |
| `TRANSLATION_INVALID_LOCALE` | Invalid BCP 47 code in filename | Use a valid locale code (e.g., `fr`, `de`, `pt-BR`) |
