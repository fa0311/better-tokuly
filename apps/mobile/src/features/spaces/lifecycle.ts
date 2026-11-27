import type { AccountLifecycle } from "#/seams/lifecycle";
import { leaveAs, useAudioSession } from "./session";

/**
 * The room or hangout an account is inside ends with it: the microphone and
 * the sound stop, and the server hears that it left, with its own API. Why it
 * was put outside is not told to the next account.
 */
export default {
  beforeLeave: leaveAs,
  forget: () => useAudioSession.setState({ active: null, exit: null }),
} satisfies AccountLifecycle;
