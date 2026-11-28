import { thumbHashToDataURL } from "thumbhash";

const decoded = new Map<string, string>();

/**
 * The blurred stand-in of a picture from its ThumbHash (base64), as a `data:`
 * URL; empty when there is none or it cannot be read. Remembered, since the
 * same locked moment is drawn again and again.
 */
export function blurUrl(thumbhash: string): string {
  if (!thumbhash) return "";
  let url = decoded.get(thumbhash);
  if (url === undefined) {
    try {
      const bytes = Uint8Array.from(atob(thumbhash), (char) => char.charCodeAt(0));
      url = thumbHashToDataURL(bytes);
    } catch {
      url = "";
    }
    decoded.set(thumbhash, url);
  }
  return url;
}
