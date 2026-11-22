import { describe, it, expect, afterEach } from 'vitest';
import { callableFetch } from './fetch';

const realFetch = globalThis.fetch;
afterEach(() => { globalThis.fetch = realFetch; });

/** A fetch that behaves like the browser's: refuses any `this` but the global object. */
function browserLikeFetch(seen: string[]): typeof fetch {
  return function (this: unknown, input: RequestInfo | URL) {
    if (this !== undefined && this !== globalThis) throw new TypeError("Failed to execute 'fetch' on 'Window': Illegal invocation");
    seen.push(String(input));
    return Promise.resolve(new Response('{}'));
  } as typeof fetch;
}

describe('callableFetch', () => {
  it('can be stored on an object and called as a method (the global fetch, looked up at call time)', async () => {
    const seen: string[] = [];
    globalThis.fetch = browserLikeFetch(seen);
    const holder = { fetchImpl: callableFetch() };
    await holder.fetchImpl('https://a.example/x');
    expect(seen).toEqual(['https://a.example/x']);
  });

  it('wraps a given fetch the same way', async () => {
    const seen: string[] = [];
    const holder = { fetchImpl: callableFetch(browserLikeFetch(seen)) };
    await holder.fetchImpl('https://a.example/y');
    expect(seen).toEqual(['https://a.example/y']);
  });

  it('a bare fetch stored on an object fails like a browser (the bug it prevents)', () => {
    const holder = { fetchImpl: browserLikeFetch([]) };
    expect(() => holder.fetchImpl('https://a.example/z')).toThrow(/Illegal invocation/);
  });
});
