/**
 * Response headers of the pages that hand out access (`/oauth/authorize`,
 * `/oauth/device`): no other site may show them in a frame, where it could
 * lay its own picture over the allow button. The Content-Security-Policy of
 * every page says so too (`src/start.ts`); old browsers know only this header.
 */
export const NO_FRAMING: Record<string, string> = {
  "X-Frame-Options": "DENY",
  // The address carries the application's `state`: keep it out of other sites' logs.
  "Referrer-Policy": "no-referrer",
  "Cache-Control": "no-store",
};
