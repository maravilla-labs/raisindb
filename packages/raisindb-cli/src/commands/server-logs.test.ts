import fs from 'fs';
import os from 'os';
import path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { pickLogFile, rotateIfLarger } from './server-logs.js';

let dir: string;

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisindb-logs-'));
});

afterEach(() => {
  fs.rmSync(dir, { recursive: true, force: true });
});

describe('rotateIfLarger', () => {
  it('leaves a small log alone', () => {
    const f = path.join(dir, 'server.console.log');
    fs.writeFileSync(f, 'x'.repeat(10));
    expect(rotateIfLarger(f, 100, 2)).toBe(false);
    expect(fs.existsSync(`${f}.1`)).toBe(false);
  });

  it('rotates an oversized log and keeps at most `keep` copies', () => {
    const f = path.join(dir, 'server.console.log');
    for (let i = 0; i < 4; i++) {
      fs.writeFileSync(f, String(i).repeat(200));
      expect(rotateIfLarger(f, 100, 2)).toBe(true);
    }
    expect(fs.existsSync(f)).toBe(false);
    expect(fs.readFileSync(`${f}.1`, 'utf-8')[0]).toBe('3');
    expect(fs.readFileSync(`${f}.2`, 'utf-8')[0]).toBe('2');
    expect(fs.existsSync(`${f}.3`)).toBe(false);
  });

  it('never throws for a missing file', () => {
    expect(rotateIfLarger(path.join(dir, 'nope.log'), 1)).toBe(false);
  });
});

describe('pickLogFile', () => {
  it('prefers whichever log was written last', async () => {
    const server = path.join(dir, 'server.log');
    const consoleLog = path.join(dir, 'server.console.log');
    expect(pickLogFile(server, consoleLog)).toBeNull();
    fs.writeFileSync(consoleLog, 'old');
    fs.utimesSync(consoleLog, new Date(1000), new Date(1000));
    fs.writeFileSync(server, 'new');
    expect(pickLogFile(server, consoleLog)).toBe(server);
    fs.utimesSync(consoleLog, new Date(), new Date(Date.now() + 5000));
    expect(pickLogFile(server, consoleLog)).toBe(consoleLog);
  });
});
