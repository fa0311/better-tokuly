/** What the composer is opened for: a new note, a reply, a quote, a draft to go on with, or text to start from. */
export interface ComposeParams {
  replyTo?: string;
  quote?: string;
  draft?: string;
  /** Text to start from (a hashtag page offers its `#tag`). */
  text?: string;
}

/** The composer's route (a modal) for `params`. */
export function composeHref(params: ComposeParams = {}): `/notes/compose${string}` {
  const query = Object.entries(params)
    .filter(([, value]) => value)
    .map(([key, value]) => `${key}=${encodeURIComponent(value as string)}`)
    .join("&");
  return `/notes/compose${query ? `?${query}` : ""}`;
}
