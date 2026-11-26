import { clearWebData } from "#/platform/web-data";
import type { AccountLifecycle } from "#/seams/lifecycle";

/**
 * Nothing the leaving account's web screens stored (cookies, localStorage,
 * cache) is there for the next account's. The web session itself needs no
 * request: the server ends it with the app's session (signing out) and
 * refuses it while that one is parked (a switch). No request is made here,
 * so no client of the account is needed, before or after an await.
 */
export default {
  beforeLeave: () => clearWebData(),
  beforeEnd: () => clearWebData(),
} satisfies AccountLifecycle;
