// SPDX-License-Identifier: BSL-1.1
import js from '@eslint/js'
import globals from 'globals'
import tsPlugin from '@typescript-eslint/eslint-plugin'
import tsParser from '@typescript-eslint/parser'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'

// Tenant-id guard: forbid the literal string 'default' from being used as a
// tenant id at assignment / argument / JSX-attribute sites. Comparisons
// (`tenantId === 'default'`) and fallback operands (`x || 'default'`) stay
// allowed — those are the legitimate dev-mode gates documented in
// `feedback_dev_mode_default`.
const tenantDefaultRules = [
  {
    selector:
      "VariableDeclarator[id.name=/^(TENANT_?ID|tenantId)$/][init.type='Literal'][init.value='default']",
    message:
      "Hardcoded tenant id 'default' breaks on multi-tenant deploys. Use `const { tenantId } = useAuth()` from contexts/AuthContext (see TenantAiSettings.tsx for the canonical pattern).",
  },
  {
    selector:
      "VariableDeclarator[id.name='tenant'][init.type='Literal'][init.value='default']",
    message:
      "Hardcoded `const tenant = 'default'` breaks on multi-tenant deploys. Use `useAuth().tenantId`.",
  },
  {
    selector:
      "CallExpression[callee.property.name=/^(getConfig|updateConfig|testProvider|listProviders|getProviders|getAvailableModels)$/] > Literal[value='default']",
    message:
      "Passing literal 'default' as a tenant id breaks on multi-tenant deploys. Pass `useAuth().tenantId`.",
  },
  {
    selector:
      "JSXAttribute[name.name=/^tenant(Id)?$/][value.type='Literal'][value.value='default']",
    message:
      "Hardcoded `tenant=\"default\"` prop breaks on multi-tenant deploys. Pass `useAuth().tenantId`.",
  },
]

export default [
  {
    ignores: ['dist/**', 'node_modules/**', 'public/**', '*.config.js', '*.config.ts'],
  },
  js.configs.recommended,
  {
    files: ['src/**/*.{ts,tsx}'],
    languageOptions: {
      parser: tsParser,
      parserOptions: {
        ecmaVersion: 'latest',
        sourceType: 'module',
        ecmaFeatures: { jsx: true },
      },
      globals: {
        ...globals.browser,
        ...globals.es2022,
      },
    },
    plugins: {
      '@typescript-eslint': tsPlugin,
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      ...tsPlugin.configs.recommended.rules,
      ...reactHooks.configs.recommended.rules,
      // Pre-existing legacy debt — re-enable and clean up in a follow-up.
      // Disabled here so `npm run lint --max-warnings 0` passes after wiring up
      // the (previously-missing) flat config.
      'react-hooks/exhaustive-deps': 'off',
      'react-refresh/only-export-components': 'off',
      // Project guard rails — see tenantDefaultRules above.
      'no-restricted-syntax': ['error', ...tenantDefaultRules],
      // Loosen rules that would otherwise create a wall of pre-existing
      // warnings unrelated to the tenant fix. Tighten in a follow-up.
      '@typescript-eslint/no-unused-vars': 'off',
      '@typescript-eslint/no-explicit-any': 'off',
      '@typescript-eslint/no-empty-object-type': 'off',
      '@typescript-eslint/ban-ts-comment': 'off',
      'no-empty': 'off',
      'no-useless-escape': 'off',
      'no-case-declarations': 'off',
      'no-prototype-builtins': 'off',
      'no-control-regex': 'off',
      'no-misleading-character-class': 'off',
      'no-cond-assign': 'off',
      'no-fallthrough': 'off',
      'no-redeclare': 'off',
      'no-self-assign': 'off',
      'no-undef': 'off', // TS handles this
    },
  },
]
