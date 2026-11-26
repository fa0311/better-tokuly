import type { AccountLifecycle } from "#/seams/lifecycle";
import { pauseRunning } from "./posts";

/** Posts on their way stop; they go on once their account is back. */
export default { beforeLeave: pauseRunning } satisfies AccountLifecycle;
