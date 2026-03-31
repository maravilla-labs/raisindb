declare module 'chokidar' {
  import { EventEmitter } from 'events';

  interface WatchOptions {
    persistent?: boolean;
    ignored?: string | string[] | RegExp | ((path: string, stats?: unknown) => boolean);
    ignoreInitial?: boolean;
    followSymlinks?: boolean;
    cwd?: string;
    disableGlobbing?: boolean;
    usePolling?: boolean;
    interval?: number;
    binaryInterval?: number;
    alwaysStat?: boolean;
    depth?: number;
    awaitWriteFinish?: boolean | {
      stabilityThreshold?: number;
      pollInterval?: number;
    };
    ignorePermissionErrors?: boolean;
    atomic?: boolean | number;
  }

  interface FSWatcher extends EventEmitter {
    add(paths: string | readonly string[]): FSWatcher;
    unwatch(paths: string | readonly string[]): FSWatcher;
    getWatched(): Record<string, string[]>;
    close(): Promise<void>;
    on(event: 'add' | 'addDir' | 'change', listener: (path: string, stats?: unknown) => void): this;
    on(event: 'unlink' | 'unlinkDir', listener: (path: string) => void): this;
    on(event: 'error', listener: (error: Error) => void): this;
    on(event: 'ready', listener: () => void): this;
    on(event: 'all', listener: (eventName: string, path: string, stats?: unknown) => void): this;
  }

  function watch(paths: string | readonly string[], options?: WatchOptions): FSWatcher;

  export { FSWatcher, WatchOptions, watch };
  export default { watch };
}
