import { chatTimes } from "@tokuly/core/chat";
import i18n from "#/lib/i18n";

export { fileSize, mentionAt, playTime, type Token, tokenize } from "@tokuly/core/chat";

/** Chat's times (`@tokuly/core/chat`) in the web app's language. */
export const { clock, dayLabel, listTime, dateTime } = chatTimes(i18n);
