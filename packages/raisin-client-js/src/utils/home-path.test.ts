import { describe, expect, it } from 'vitest';
import { normalizeHomePath } from './home-path';

describe('normalizeHomePath', () => {
  it('strips the leading raisin:access_control workspace prefix', () => {
    expect(normalizeHomePath('/raisin:access_control/users/internal/alice')).toBe(
      '/users/internal/alice',
    );
  });

  it('returns workspace-relative paths unchanged', () => {
    expect(normalizeHomePath('/users/internal/alice')).toBe('/users/internal/alice');
  });

  it('trims trailing slashes', () => {
    expect(normalizeHomePath('/users/internal/alice/')).toBe('/users/internal/alice');
    expect(normalizeHomePath('/raisin:access_control/users/alice/')).toBe('/users/alice');
  });

  it('handles the bare workspace prefix', () => {
    expect(normalizeHomePath('/raisin:access_control')).toBe('/');
  });

  it('does not strip a path that merely starts with a similar segment', () => {
    expect(normalizeHomePath('/raisin:access_controlx/users')).toBe(
      '/raisin:access_controlx/users',
    );
  });

  it('returns null for empty input', () => {
    expect(normalizeHomePath(null)).toBeNull();
    expect(normalizeHomePath(undefined)).toBeNull();
    expect(normalizeHomePath('')).toBeNull();
    expect(normalizeHomePath('   ')).toBeNull();
  });
});
