import { errorMessage as sharedErrorMessage } from "#/lib/errors";

/** The text for an error shown in chat: chat's own wording for a reason comes before any other feature's. */
export function errorMessage(error: unknown): string {
  return sharedErrorMessage(error, { namespace: "chat" });
}
