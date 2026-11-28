import { useQuery } from "@tanstack/react-query";
import { chat } from "@tokuly/api";
import * as dev from "@tokuly/api/gen/tokuly/dev/v1/dev_pb.js";
import { miniAppQueries } from "@tokuly/core/miniapps";
import { api } from "#/lib/api";
import i18n from "#/lib/i18n";

export { dev };

export const NS = "miniapps";

export const miniAppClient = () => api().service(dev.MiniAppService);
export const reviewClient = () => api().service(dev.MiniAppReviewService);
export const messageClient = () => api().service(chat.MessageService);

/** An approved mini app as the caller opens it, with what they granted it. */
export function useMiniApp(appId: string) {
  return useQuery({ ...miniAppQueries.app(miniAppClient, appId), enabled: Boolean(appId) });
}

/** A text the server sends in both languages, in the one the app is showing. */
export function localized(text: { ja: string; en: string } | undefined): string {
  if (!text) return "";
  return i18n.language.startsWith("ja") ? text.ja || text.en : text.en || text.ja;
}
