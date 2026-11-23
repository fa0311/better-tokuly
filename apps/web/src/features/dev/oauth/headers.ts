/**
 * Response headers of the pages that hand out access (`/oauth/authorize`,
 * `/oauth/device`): no other site may show them in a frame, where it could
 * lay its own picture over the allow button. Both headers say the same
 * thing; old browsers know only the first.
 */
export const NO_FRAMING: Record<string, string> = {
  "X-Frame-Options": "DENY",
  "Content-Security-Policy": "frame-ancestors 'none'",
  // The address carries the application's `state`: keep it out of other sites' logs.
  "Referrer-Policy": "no-referrer",
  "Cache-Control": "no-store",
};
