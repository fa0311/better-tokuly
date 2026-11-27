import type { AccountLifecycle } from "#/seams/lifecycle";
import { useLive } from "./live";

/** Who was typing to the account, and how far its conversations were read, are not the next one's. */
export default { forget: () => useLive.getState().reset() } satisfies AccountLifecycle;
