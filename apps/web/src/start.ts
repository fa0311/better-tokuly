import { createCsrfMiddleware, createMiddleware, createStart } from "@tanstack/react-start";

/**
 * Pages that are meant to be shown inside other sites. The live area's OBS
 * widgets (`/overlay/<token>`, linked by the API) are browser sources that
 * streaming tools may also put in a frame; nothing else is.
 */
const EMBEDDABLE = ["/overlay/"];

/**
 * No page of the app may be framed by another site, unless it is made to be
 * embedded: a page with buttons inside a hostile frame can be clicked
 * through a decoy (clickjacking). Headers a route sets itself (the OAuth
 * pages send stricter ones) are kept.
 */
const framing = createMiddleware().server(async ({ next, pathname }) => {
  const result = await next();
  const headers = result.response.headers;
  if (EMBEDDABLE.some((prefix) => pathname.startsWith(prefix))) {
    if (!headers.has("content-security-policy"))
      headers.set("content-security-policy", "frame-ancestors *");
    return result;
  }
  if (!headers.has("content-security-policy"))
    headers.set("content-security-policy", "frame-ancestors 'none'");
  if (!headers.has("x-frame-options")) headers.set("x-frame-options", "DENY");
  return result;
});

/**
 * Declaring a start instance replaces Start's default middleware, so the
 * CSRF check of server functions is listed here explicitly.
 */
const csrf = createCsrfMiddleware({ filter: (context) => context.handlerType === "serverFn" });

export const startInstance = createStart(() => ({ requestMiddleware: [csrf, framing] }));
