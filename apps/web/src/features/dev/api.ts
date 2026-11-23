import type { common } from "@tokuly/api";
import * as dev from "@tokuly/api/gen/tokuly/dev/v1/dev_pb.js";
import { api } from "#/lib/api";
import i18n from "#/lib/i18n";

/** The generated messages and services of the developer platform (`tokuly.dev.v1`). */
export { dev };

export const tokenClient = () => api().service(dev.TokenService);
export const appClient = () => api().service(dev.AppService);
export const authorizationClient = () => api().service(dev.AuthorizationService);
export const botClient = () => api().service(dev.BotService);
export const webhookClient = () => api().service(dev.WebhookService);
export const referenceClient = () => api().service(dev.ReferenceService);
export const usageClient = () => api().service(dev.UsageService);

/**
 * Query keys of this area; all start with `dev`.
 *
 * Nothing secret is ever cached under them: tokens, client secrets and
 * signing secrets exist in the response of the call that made them and in
 * the dialog that shows them, nowhere else (see `secret-dialog.tsx`).
 */
export const keys = {
  all: ["dev"] as const,
  scopes: ["dev", "scopes"] as const,
  tokens: ["dev", "tokens"] as const,
  apps: ["dev", "apps"] as const,
  app: (appId: string) => ["dev", "app", appId] as const,
  appUsers: (appId: string) => ["dev", "app", appId, "users"] as const,
  authorized: ["dev", "authorized"] as const,
  bots: ["dev", "bots"] as const,
  bot: (botId: string) => ["dev", "bot", botId] as const,
  botTokens: (botId: string) => ["dev", "bot", botId, "tokens"] as const,
  eventTypes: ["dev", "event-types"] as const,
  /** The caller's own webhooks under the empty bot ID. */
  webhooks: (botId: string) => ["dev", "webhooks", botId] as const,
  webhook: (webhookId: string) => ["dev", "webhook", webhookId] as const,
  deliveries: (webhookId: string, status: dev.DeliveryStatus) =>
    ["dev", "webhook", webhookId, "deliveries", status] as const,
  delivery: (deliveryId: string) => ["dev", "delivery", deliveryId] as const,
  usage: (kind: UsageSubject["kind"], id: string, days: number) =>
    ["dev", "usage", kind, id, days] as const,
  services: ["dev", "reference", "services"] as const,
  method: (path: string) => ["dev", "reference", "method", path] as const,
};

/** What usage statistics can be asked about. */
export interface UsageSubject {
  kind: "token" | "app" | "bot";
  id: string;
}

/** How many rows a list asks for at a time. */
export const PAGE_SIZE = 30;
export const nextCursorOf = (page: { page?: common.PageInfo }) =>
  page.page?.nextCursor || undefined;

/** A text the server sends in both languages, in the one the app is showing. */
export function localized(text: common.LocalizedText | undefined): string {
  if (!text) return "";
  return i18n.language.startsWith("ja") ? text.ja || text.en : text.en || text.ja;
}

/** The scope catalogue rarely changes: one request per visit is plenty. */
export const scopesQuery = {
  queryKey: keys.scopes,
  queryFn: async () => (await tokenClient().listScopes({})).scopes,
  staleTime: 10 * 60_000,
};

export const eventTypesQuery = {
  queryKey: keys.eventTypes,
  queryFn: async () => (await webhookClient().listEventTypes({})).eventTypes,
  staleTime: 10 * 60_000,
};
