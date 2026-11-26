import { createCsrfMiddleware, createMiddleware, createStart } from "@tanstack/react-start";
import { contentSecurityPolicy, deploymentFrom, permissionsPolicy } from "#/lib/security-headers";

/**
 * Pages that are meant to be shown inside other sites. The live area's OBS
 * widgets (`/overlay/<token>`, linked by the API) are browser sources that
 * streaming tools may also put in a frame; nothing else is.
 */
const EMBEDDABLE = ["/overlay/"];

const deployment = deploymentFrom(process.env);

/**
 * Every page gets its Content-Security-Policy and Permissions-Policy
 * (`#/lib/security-headers`). The nonce is new for each response; the router
 * puts it on the scripts it renders (`src/router.tsx`). No page may be framed
 * by another site unless it is made to be embedded.
 */
const securityHeaders = createMiddleware().server(async ({ next, pathname }) => {
  const nonce = Buffer.from(crypto.getRandomValues(new Uint8Array(16))).toString("base64");
  const result = await next({ context: { nonce } });
  const headers = result.response.headers;
  const embeddable = EMBEDDABLE.some((prefix) => pathname.startsWith(prefix));
  headers.set("content-security-policy", contentSecurityPolicy(deployment, nonce, { embeddable }));
  headers.set("permissions-policy", permissionsPolicy({ embeddable }));
  // The same for browsers that only know the older header.
  if (!embeddable) headers.set("x-frame-options", "DENY");
  return result;
});

/**
 * Declaring a start instance replaces Start's default middleware, so the
 * CSRF check of server functions is listed here explicitly.
 */
const csrf = createCsrfMiddleware({ filter: (context) => context.handlerType === "serverFn" });

export const startInstance = createStart(() => ({ requestMiddleware: [csrf, securityHeaders] }));
