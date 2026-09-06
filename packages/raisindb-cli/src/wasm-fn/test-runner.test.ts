import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import {
  defaultTestCommand,
  loadServerCases,
  matchesExpectation,
  serverCasesFile,
} from './test-runner.js';

let dir: string;

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), 'raisin-tests-'));
});

afterEach(() => {
  fs.rmSync(dir, { recursive: true, force: true });
});

describe('defaultTestCommand', () => {
  it('uses each language’s own runner', () => {
    expect(defaultTestCommand('rust')).toBe('cargo test');
    expect(defaultTestCommand('go')).toBe('go test ./...');
    expect(defaultTestCommand('ts')).toBe('npm test');
  });
});

describe('loadServerCases', () => {
  it('is empty when the project has no cases', () => {
    expect(loadServerCases(dir)).toEqual([]);
  });

  it('reads the scaffold shape, handler included', () => {
    fs.mkdirSync(path.dirname(serverCasesFile(dir)), { recursive: true });
    fs.writeFileSync(
      serverCasesFile(dir),
      JSON.stringify([{ handler: 'shout', input: { name: 'Ada' }, expect: { greeting: 'HI' } }])
    );
    expect(loadServerCases(dir)).toEqual([
      { handler: 'shout', input: { name: 'Ada' }, expect: { greeting: 'HI' } },
    ]);
  });

  it('names the file when the JSON is broken', () => {
    fs.mkdirSync(path.dirname(serverCasesFile(dir)), { recursive: true });
    fs.writeFileSync(serverCasesFile(dir), '{');
    expect(() => loadServerCases(dir)).toThrow(/server\.json: not valid JSON/);
  });

  it('rejects a non-array document', () => {
    fs.mkdirSync(path.dirname(serverCasesFile(dir)), { recursive: true });
    fs.writeFileSync(serverCasesFile(dir), '{"input":{}}');
    expect(() => loadServerCases(dir)).toThrow(/expected an array/);
  });
});

describe('matchesExpectation', () => {
  it('matches an object as a subset, so extra fields do not fail a case', () => {
    expect(matchesExpectation({ greeting: 'hi', abi: '0.1.0' }, { greeting: 'hi' })).toBe(true);
  });

  it('fails on a differing value', () => {
    expect(matchesExpectation({ greeting: 'hi' }, { greeting: 'ho' })).toBe(false);
  });

  it('fails on a missing key', () => {
    expect(matchesExpectation({ abi: '0.1.0' }, { greeting: 'hi' })).toBe(false);
  });

  it('matches arrays exactly, element for element', () => {
    expect(matchesExpectation([1, 2], [1, 2])).toBe(true);
    expect(matchesExpectation([1, 2, 3], [1, 2])).toBe(false);
  });

  it('recurses into nested objects', () => {
    expect(matchesExpectation({ a: { b: 1, c: 2 } }, { a: { b: 1 } })).toBe(true);
    expect(matchesExpectation({ a: { b: 1 } }, { a: { b: 2 } })).toBe(false);
  });

  it('accepts anything when a case asserts nothing', () => {
    expect(matchesExpectation({ any: true }, undefined)).toBe(true);
  });

  it('compares primitives and null strictly', () => {
    expect(matchesExpectation(null, null)).toBe(true);
    expect(matchesExpectation(0, null)).toBe(false);
    expect(matchesExpectation('a', 'a')).toBe(true);
  });
});
