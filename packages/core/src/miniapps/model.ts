import type { common } from "@tokuly/api";
import {
  type Call,
  type Card,
  type ErrorCode,
  type Person,
  type Scope,
  scopeOf,
} from "@tokuly/miniapp/protocol";

/** Where a mini app opens inside Tokuly, and what it opens with. */
export interface MiniAppLink {
  appId: string;
  /** A card's path on the mini app's origin; empty for its launch URL. */
  path: string;
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * The page that runs a mini app: `/miniapps/<id>`, with the card's path
 * (`p`), the conversation it was opened from (`c`) and the approved origin
 * (`o`), which the page's Content-Security-Policy allows as its only frame.
 */
export function miniAppHref(
  appId: string,
  {
    path = "",
    conversationId = "",
    origin = "",
  }: { path?: string; conversationId?: string; origin?: string } = {},
): string {
  const query = new URLSearchParams();
  if (path) query.set("p", path);
  if (conversationId) query.set("c", conversationId);
  if (origin) query.set("o", origin);
  const search = query.toString();
  return `/miniapps/${appId}${search ? `?${search}` : ""}`;
}

/**
 * Reads a link to a mini app in a message: `<webOrigin>/miniapps/<id>[?p=…]`
 * or the same path alone. Anything else (another site, another page) is not one.
 */
export function readMiniAppLink(href: string, webOrigin: string): MiniAppLink | undefined {
  let url: URL;
  try {
    url = new URL(href, webOrigin);
  } catch {
    return undefined;
  }
  if (url.origin !== new URL(webOrigin).origin) return undefined;
  const match = /^\/miniapps\/([^/]+)\/?$/.exec(url.pathname);
  const appId = match?.[1] ?? "";
  if (!UUID.test(appId)) return undefined;
  return { appId: appId.toLowerCase(), path: url.searchParams.get("p") ?? "" };
}

/** The message a shared card becomes: its words, then the link that opens it. */
export function cardMessage(card: Card, webOrigin: string, appId: string): string {
  const link = `${new URL(webOrigin).origin}${miniAppHref(appId, { path: card.path === "/" ? "" : card.path })}`;
  return [card.title, card.text, link].filter(Boolean).join("\n");
}

/** A person as a mini app sees them: never more than these four fields. */
export function personOf(user: common.UserSummary): Person {
  return {
    id: user.id,
    name: user.displayName || user.handle,
    handle: user.handle,
    avatarUrl: user.avatar?.url ?? "",
  };
}

/** What a host does with a call before running it. */
export type Step =
  | { kind: "run" }
  /** Ask the person to grant this scope first (once per mini app). */
  | { kind: "ask"; scope: Scope }
  | { kind: "refuse"; code: ErrorCode; message: string };

/**
 * Decides whether a call may run: its scope must be one the approved manifest
 * asks for (`requested`) and one the person granted, and a card needs a
 * conversation to go to.
 */
export function nextStep(
  call: Call,
  {
    requested,
    granted,
    inConversation,
  }: { requested: string[]; granted: string[]; inConversation: boolean },
): Step {
  if (call.method === "shareCard" && !inConversation)
    return {
      kind: "refuse",
      code: "NOT_IN_CONVERSATION",
      message: "open the mini app from a conversation to share",
    };
  const scope = scopeOf(call);
  if (!scope) return { kind: "run" };
  if (!requested.includes(scope))
    return {
      kind: "refuse",
      code: "SCOPE_NOT_REQUESTED",
      message: `the manifest does not ask for ${scope}`,
    };
  return granted.includes(scope) ? { kind: "run" } : { kind: "ask", scope };
}
