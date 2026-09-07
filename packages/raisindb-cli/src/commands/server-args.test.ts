import { describe, expect, it } from 'vitest';
import { buildServerArgs } from './server.js';

describe('buildServerArgs', () => {
  it('forwards --port (the binary rejects --http-port) and enables pgwire by default', () => {
    const { args, pgwireEnabled } = buildServerArgs([], { devMode: true, port: '8081', env: {} });
    expect(args).toEqual(['--dev-mode', '--port', '8081', '--pgwire-enabled', 'true', '--pgwire-port', '5432']);
    expect(args).not.toContain('--http-port');
    expect(pgwireEnabled).toBe(true);
  });

  it('uses the requested pgwire port', () => {
    const { args } = buildServerArgs([], { devMode: false, pgwirePort: '5490', env: {} });
    expect(args).toEqual(['--pgwire-enabled', 'true', '--pgwire-port', '5490']);
  });

  it('lets a config file decide pgwire and does not claim it in the banner', () => {
    const { args, pgwireEnabled } = buildServerArgs(['--config', 'x.toml'], { devMode: true, env: {} });
    expect(args).toEqual(['--config', 'x.toml', '--dev-mode']);
    expect(pgwireEnabled).toBe(false);
  });

  it('respects an explicit pass-through --pgwire-enabled false', () => {
    const { args, pgwireEnabled } = buildServerArgs(['--pgwire-enabled', 'false'], { devMode: true, env: {} });
    expect(args).toEqual(['--pgwire-enabled', 'false', '--dev-mode']);
    expect(pgwireEnabled).toBe(false);
  });

  it('respects RAISIN_PGWIRE_ENABLED from the environment', () => {
    const on = buildServerArgs([], { devMode: true, env: { RAISIN_PGWIRE_ENABLED: 'true' } });
    expect(on.args).toEqual(['--dev-mode']);
    expect(on.pgwireEnabled).toBe(true);
    const off = buildServerArgs([], { devMode: true, env: { RAISIN_PGWIRE_ENABLED: 'false' } });
    expect(off.pgwireEnabled).toBe(false);
  });

  it('does not duplicate flags the user already passed through', () => {
    const { args } = buildServerArgs(['--dev-mode', '--port', '9000'], { devMode: true, port: '8080', env: {} });
    expect(args.filter((a) => a === '--dev-mode')).toHaveLength(1);
    expect(args.filter((a) => a === '--port')).toHaveLength(1);
    expect(args).toContain('9000');
  });
});
