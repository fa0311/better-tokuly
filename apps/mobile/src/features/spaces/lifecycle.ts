import type { AccountLifecycle } from "#/seams/lifecycle";
import { leaveAs } from "./session";

/**
 * The room or hangout an account is inside ends with it: the microphone and
 * the sound stop, and the server hears that it left, with its own API.
 */
export default { beforeLeave: leaveAs } satisfies AccountLifecycle;
