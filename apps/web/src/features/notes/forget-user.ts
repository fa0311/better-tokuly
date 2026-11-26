import { forgetUserNotes } from "@tokuly/core/notes";
import type { ForgetUser } from "#/seams/forget-user";

/** After an unfriend or a block: their notes leave every loaded timeline. */
export default forgetUserNotes satisfies ForgetUser;
