import { forgetUserNotes } from "@tokuly/core/notes";
import type { ForgetUser } from "#/seams/forget-user";

/** A friendship ended: their notes leave every loaded timeline. */
export default forgetUserNotes satisfies ForgetUser;
