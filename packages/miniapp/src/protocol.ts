/**
 * The messages between a mini app and Tokuly (`docs/miniapps.md`).
 *
 * A mini app sends a request (`{ protocol, id, call }`); Tokuly answers it
 * once (`{ protocol, id, ok, result | error }`). Both sides read what they
 * receive with the functions below and drop anything else: a message that is
 * not shaped like the protocol is not an error, it is somebody else's.
 *
 * Platform-neutral: the hosts (web, mobile) and the SDK all use it.
 */

export const PROTOCOL = "tokuly.miniapp.v1";

/** What a mini app may be granted: `profile` (who the person is), `friends` (friends they pick). */
export type Scope = "profile" | "friends";

export const SCOPES: readonly Scope[] = ["profile", "friends"];

/** A person as a mini app sees them. */
export interface Person {
  id: string;
  name: string;
  handle: string;
  /** Empty when they have no picture. */
  avatarUrl: string;
}

/** A card shared to the conversation the mini app was opened from. */
export interface Card {
  /** 1-60 characters. */
  title: string;
  /** Up to 200 characters. */
  text: string;
  /** Where the card opens the mini app: a path on its own origin (`/`, `/?total=1200`). */
  path: string;
}

/** What a mini app learns when it starts. */
export interface Launch {
  /** The path the mini app was opened at (a card's `path`, or the launch URL's). */
  path: string;
  language: "ja" | "en";
  /** Whether a card can be shared: the mini app was opened from a conversation. */
  inConversation: boolean;
  grantedScopes: Scope[];
}

export type Call =
  | { method: "hello" }
  | { method: "getProfile" }
  | { method: "pickFriends"; max: number }
  | { method: "shareCard"; card: Card }
  | { method: "close" };

export type Method = Call["method"];

/** What each call resolves to. */
export interface Results {
  hello: Launch;
  getProfile: Person;
  pickFriends: Person[];
  shareCard: { shared: true };
  close: { closed: true };
}

export type ErrorCode =
  /** The message is a request but breaks a rule (a field, a limit). */
  | "INVALID_REQUEST"
  | "UNKNOWN_METHOD"
  /** The approved manifest does not ask for the scope this call needs. */
  | "SCOPE_NOT_REQUESTED"
  /** The person did not grant the scope. */
  | "CONSENT_DENIED"
  /** The person closed the picker or the share sheet. */
  | "CANCELLED"
  /** `shareCard` outside a conversation. */
  | "NOT_IN_CONVERSATION"
  /** Another call is waiting for the person. */
  | "BUSY"
  | "UNAVAILABLE";

export interface BridgeError {
  code: ErrorCode;
  message: string;
}

export interface Request {
  protocol: typeof PROTOCOL;
  id: string;
  call: Call;
}

export type Response =
  | { protocol: typeof PROTOCOL; id: string; ok: true; result: unknown }
  | { protocol: typeof PROTOCOL; id: string; ok: false; error: BridgeError };

export const LIMITS = {
  titleChars: 60,
  textChars: 200,
  pathChars: 500,
  idChars: 64,
  /** Friends one `pickFriends` may return. */
  friends: 20,
} as const;

/** The scope a call needs, if any. */
export function scopeOf(call: Call): Scope | undefined {
  if (call.method === "getProfile") return "profile";
  if (call.method === "pickFriends") return "friends";
  return undefined;
}

type Fields = Record<string, unknown>;

const isRecord = (value: unknown): value is Fields =>
  typeof value === "object" && value !== null && !Array.isArray(value);

// Control characters (other than a line break in a card's text).
// biome-ignore lint/suspicious/noControlCharactersInRegex: that is what it finds
const CONTROL = /[\u0000-\u0009\u000b-\u001f\u007f]/;

const chars = (text: string) => [...text].length;

/**
 * Reads what a host received. `undefined` when it is not a request of this
 * protocol (ignore it); `{ error }` for a request that breaks a rule (answer
 * it with the error when it has an `id`).
 */
export function readRequest(
  data: unknown,
): { id: string; call: Call } | { id: string; error: BridgeError } | undefined {
  if (!isRecord(data) || data.protocol !== PROTOCOL) return undefined;
  const { id, call } = data;
  if (typeof id !== "string" || id.length === 0 || id.length > LIMITS.idChars) return undefined;
  const invalid = (message: string) => ({
    id,
    error: { code: "INVALID_REQUEST" as const, message },
  });
  if (!isRecord(call) || typeof call.method !== "string") return invalid("call.method is missing");
  switch (call.method) {
    case "hello":
    case "getProfile":
    case "close":
      return { id, call: { method: call.method } };
    case "pickFriends": {
      const max = call.max ?? LIMITS.friends;
      if (typeof max !== "number" || !Number.isInteger(max) || max < 1 || max > LIMITS.friends)
        return invalid(`max must be 1-${LIMITS.friends}`);
      return { id, call: { method: "pickFriends", max } };
    }
    case "shareCard": {
      const card = readCard(call.card);
      return typeof card === "string" ? invalid(card) : { id, call: { method: "shareCard", card } };
    }
    default:
      return { id, error: { code: "UNKNOWN_METHOD", message: `unknown method ${call.method}` } };
  }
}

/** A card, or what is wrong with it. */
function readCard(value: unknown): Card | string {
  if (!isRecord(value)) return "card is missing";
  const title = typeof value.title === "string" ? value.title.trim() : "";
  const text = typeof value.text === "string" ? value.text.trim() : "";
  const path = value.path === undefined ? "/" : value.path;
  if (
    chars(title) < 1 ||
    chars(title) > LIMITS.titleChars ||
    CONTROL.test(title) ||
    title.includes("\n")
  )
    return `card.title must be 1-${LIMITS.titleChars} characters on one line`;
  if (chars(text) > LIMITS.textChars || CONTROL.test(text))
    return `card.text must be at most ${LIMITS.textChars} characters`;
  if (!isCardPath(path)) return "card.path must be a path on the mini app's origin";
  return { title, text, path };
}

/** A path on the mini app's own origin: `/…`, never `//host` or a scheme. */
export function isCardPath(path: unknown): path is string {
  return (
    typeof path === "string" &&
    path.length <= LIMITS.pathChars &&
    path.startsWith("/") &&
    !path.startsWith("//") &&
    !path.includes("\\") &&
    !CONTROL.test(path)
  );
}

/**
 * The page a card opens: its path on the launch URL's origin. `undefined`
 * when the path would leave that origin.
 */
export function cardTarget(launchUrl: string, path: string): string | undefined {
  if (!isCardPath(path)) return undefined;
  try {
    const launch = new URL(launchUrl);
    const target = new URL(path, launch.origin);
    return target.origin === launch.origin ? target.href : undefined;
  } catch {
    return undefined;
  }
}

/** Reads what the SDK received: an answer of this protocol, or `undefined`. */
export function readResponse(data: unknown): Response | undefined {
  if (!isRecord(data) || data.protocol !== PROTOCOL || typeof data.id !== "string")
    return undefined;
  if (data.ok === true) return { protocol: PROTOCOL, id: data.id, ok: true, result: data.result };
  if (data.ok === false && isRecord(data.error) && typeof data.error.code === "string")
    return {
      protocol: PROTOCOL,
      id: data.id,
      ok: false,
      error: { code: data.error.code as ErrorCode, message: String(data.error.message ?? "") },
    };
  return undefined;
}

export const answer = (id: string, result: unknown): Response => ({
  protocol: PROTOCOL,
  id,
  ok: true,
  result,
});

export const refuse = (id: string, code: ErrorCode, message: string): Response => ({
  protocol: PROTOCOL,
  id,
  ok: false,
  error: { code, message },
});
