/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_RAISIN_URL?: string;
  readonly VITE_PORT?: string;
  readonly VITE_NO_OPEN?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
