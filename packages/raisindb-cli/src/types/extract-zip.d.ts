declare module 'extract-zip' {
  interface ExtractOptions {
    dir: string;
    defaultDirMode?: number;
    defaultFileMode?: number;
    onEntry?: (entry: { fileName: string }, zipfile: unknown) => void;
  }

  function extractZip(zipPath: string, opts: ExtractOptions): Promise<void>;

  export = extractZip;
}
