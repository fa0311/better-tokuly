import type { AnyRouter } from "@tanstack/react-router";

/**
 * The app's router, handed over by the chat background component (mounted
 * for as long as someone is signed in), so plain functions can navigate.
 */
let router: AnyRouter | null = null;

export function setChatRouter(instance: AnyRouter | null): void {
  router = instance;
}

/**
 * Goes to the direct conversation with a person, given their handle
 * (`aoi`, `@aoi`) or user ID. The conversation is created when the two never
 * talked; when it cannot be opened (unknown person, their privacy settings)
 * the chat list is shown with a message saying why.
 */
export function openDirectChat(handleOrUserId: string): Promise<void> {
  if (!router) return Promise.reject(new Error("openDirectChat needs a signed-in app"));
  return router.navigate({ to: "/chat", search: { user: handleOrUserId.replace(/^@/, "") } });
}
