import type { ForgetUser } from "#/seams/forget-user";
import { forgetUserNotes } from "./api";

/** After an unfriend or a block: their notes leave every loaded timeline. */
export default forgetUserNotes satisfies ForgetUser;
