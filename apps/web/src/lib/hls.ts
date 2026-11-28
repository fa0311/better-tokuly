import { loadNow } from "./load-module";

/**
 * hls.js, loaded the first time a stream plays where the browser has no HLS
 * of its own: it is large. Rejects when its code does not arrive (`loadNow`):
 * the player falls back to the MP4 or shows that it failed.
 */
export function loadHls(): Promise<typeof import("hls.js")> {
  return loadNow(() => import("hls.js"));
}
