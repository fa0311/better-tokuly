import { clearWebData } from "#/platform/web-data";
import type { AccountLifecycle } from "#/seams/lifecycle";
import { clearUnusedWebData } from "./open-screens";

/**
 * Nothing the leaving account's web screens stored (cookies, localStorage,
 * cache) is there for the next account's. The web session itself needs no
 * request: the server ends it with the app's session (signing out) and
 * refuses it while that one is parked (a switch). No request is made here,
 * so no client of the account is needed, before or after an await.
 *
 * A departed session ends in the background, possibly while the account in
 * use has a web screen open: that one's data is its own (it started empty,
 * after the departed account left) and stays.
 */
export default {
  beforeLeave: () => clearWebData(),
  beforeEnd: () => clearUnusedWebData(),
} satisfies AccountLifecycle;
