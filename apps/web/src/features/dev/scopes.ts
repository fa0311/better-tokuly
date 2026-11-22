/**
 * Scopes as the forms handle them. A scope is `<domain>:read` or
 * `<domain>:write`, and writing includes reading, so a form offers one choice
 * per domain: nothing, read, or read and write.
 */

/** What the catalogue says about one scope (the fields of `ScopeInfo` used here). */
export interface ScopeEntry {
  scope: string;
  domain: string;
  write: boolean;
  sensitive: boolean;
}

export type ScopeLevel = "none" | "read" | "write";

/** One domain of the catalogue with the levels it offers. */
export interface ScopeGroup<T extends ScopeEntry = ScopeEntry> {
  domain: string;
  read?: T;
  write?: T;
  sensitive: boolean;
}

/** The domain whose scopes carry location: shown on its own, with a warning. */
export const LOCATION_DOMAIN = "geo";
/** The scope everything that spends coins needs, whatever its own domain. */
export const SPEND_SCOPE = "wallet:write";

/** The catalogue by domain, in the catalogue's order. */
export function groupScopes<T extends ScopeEntry>(catalogue: readonly T[]): ScopeGroup<T>[] {
  const groups = new Map<string, ScopeGroup<T>>();
  for (const entry of catalogue) {
    const group = groups.get(entry.domain) ?? { domain: entry.domain, sensitive: false };
    if (entry.write) group.write = entry;
    else group.read = entry;
    group.sensitive ||= entry.sensitive;
    groups.set(entry.domain, group);
  }
  return [...groups.values()];
}

/** The level a selection gives a domain. */
export function levelOf(selected: readonly string[], domain: string): ScopeLevel {
  if (selected.includes(`${domain}:write`)) return "write";
  if (selected.includes(`${domain}:read`)) return "read";
  return "none";
}

/** The selection with one domain set to a level (one scope per domain: write includes read). */
export function withLevel(selected: readonly string[], domain: string, level: ScopeLevel) {
  const others = selected.filter((scope) => scope.split(":")[0] !== domain);
  return level === "none" ? others : [...others, `${domain}:${level}`];
}

/** Whether a selection can write somewhere but could not pay for anything. */
export function writesWithoutSpending(selected: readonly string[]): boolean {
  return (
    !selected.includes(SPEND_SCOPE) &&
    selected.some((scope) => scope.endsWith(":write") && scope !== SPEND_SCOPE)
  );
}

/**
 * Whether a bot token may hold a scope. Bots are read-only for now, and never
 * see the wallet: a bot is an account anyone can make more of.
 */
export function allowedForBots(entry: ScopeEntry): boolean {
  return !entry.write && entry.domain !== "wallet";
}

/** The scopes named in a space-separated list (`ApiMethod.scope`, `WebhookEventType.scope`). */
export function splitScopes(list: string): string[] {
  return list.split(/\s+/).filter(Boolean);
}
