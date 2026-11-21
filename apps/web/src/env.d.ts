/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Base URL of the vector tile service (default `/tiles`). */
  readonly VITE_TILES_URL?: string;
  /** Base URL of basemap fonts and sprites (default `/map-assets`). */
  readonly VITE_MAP_ASSETS_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
