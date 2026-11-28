import { forgetUserMoments } from "@tokuly/core/moments";
import type { ForgetUser } from "#/seams/forget-user";

/** A friendship ended: their moments leave the friends' list and the map. */
export default forgetUserMoments satisfies ForgetUser;
