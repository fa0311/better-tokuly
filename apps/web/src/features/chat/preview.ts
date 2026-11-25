import { chatSentences } from "@tokuly/core/chat";
import i18n from "#/lib/i18n";

/** Sentences built from messages (`@tokuly/core/chat`) in the web app's language. */
export const {
  nameOf,
  systemText,
  callText,
  attachmentsText,
  contentText,
  previewText,
  replyText,
} = chatSentences(i18n);
