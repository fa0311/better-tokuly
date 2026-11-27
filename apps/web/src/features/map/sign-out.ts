import type { SignOutCleanup } from "#/seams/sign-out";
import { LAST_VIEW_KEY } from "./last-view";

/** Where the map was left. (Owed stops of sharing stay, by account, until the server has them.) */
export default { storage: [LAST_VIEW_KEY] } satisfies SignOutCleanup;
