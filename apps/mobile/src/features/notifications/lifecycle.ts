import { notify } from "@tokuly/api";
import type { AccountLifecycle } from "#/seams/lifecycle";
import { leavePush } from "./push";

/** No push of an account reaches the phone once it is in someone else's hands. */
export default {
  beforeLeave: ({ sessionId }) => leavePush(sessionId),
  beforeEnd: (session) => session.service(notify.PushService).unregisterDevice({ deviceId: "" }),
} satisfies AccountLifecycle;
