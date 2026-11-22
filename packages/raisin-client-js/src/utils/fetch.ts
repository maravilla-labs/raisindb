/**
 * A `fetch` that may be stored on an object and called as a method.
 *
 * Browsers refuse `window.fetch` called with any `this` but the window ("Failed to execute 'fetch' on 'Window':
 * Illegal invocation"), so `this.fetchImpl = fetch; this.fetchImpl(url)` throws in a browser and works in Node,
 * which is how it slipped past tests. The returned function calls the given one (or the global one, looked up at
 * call time) as a plain function.
 */
export function callableFetch(given?: typeof fetch): typeof fetch {
  if (given) return ((input: RequestInfo | URL, init?: RequestInit) => given(input, init)) as typeof fetch;
  return ((input: RequestInfo | URL, init?: RequestInit) => globalThis.fetch(input, init)) as typeof fetch;
}
