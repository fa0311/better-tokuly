import type { notes } from "@tokuly/api";

/** Which list of notes a timeline shows. */
export type TimelineSource =
  | { kind: "home" }
  | { kind: "following" }
  | { kind: "global" }
  | { kind: "mentions" }
  | { kind: "user"; userId: string; filter: notes.UserTimelineFilter }
  /** `tag` is normalized (lowercase, no `#`). */
  | { kind: "hashtag"; tag: string }
  /** A note list's members. */
  | { kind: "list"; listId: string }
  /** What one of the viewer's antennas collected. */
  | { kind: "antenna"; antennaId: string };

function sourceKey(source: TimelineSource): readonly unknown[] {
  if (source.kind === "user") return ["user", source.userId, source.filter];
  if (source.kind === "hashtag") return ["hashtag", source.tag];
  if (source.kind === "list") return ["list", source.listId];
  if (source.kind === "antenna") return ["antenna", source.antennaId];
  return [source.kind];
}

/** Query keys of the notes domain. Everything that holds notes lives under `["notes"]`, which is where live updates look. */
export const notesKeys = {
  all: ["notes"] as const,
  rules: ["notes", "rules"] as const,
  timelines: ["notes", "timeline"] as const,
  timeline: (source: TimelineSource) => ["notes", "timeline", ...sourceKey(source)] as const,
  userTimelines: (userId: string) => ["notes", "timeline", "user", userId] as const,
  thread: (noteId: string) => ["notes", "thread", noteId] as const,
  threads: ["notes", "thread"] as const,
  quotes: (noteId: string) => ["notes", "quotes", noteId] as const,
  note: (noteId: string) => ["notes", "note", noteId] as const,
  likers: (noteId: string) => ["notes", "likers", noteId] as const,
  renoters: (noteId: string) => ["notes", "renoters", noteId] as const,
  trending: ["notes", "trending"] as const,
  trendingNotes: (language: string) => ["notes", "trending-notes", language] as const,
  drafts: ["notes", "drafts"] as const,
  mutes: ["notes", "mutes"] as const,
  edits: (noteId: string) => ["notes", "edits", noteId] as const,
  analytics: (noteId: string) => ["notes", "analytics", noteId] as const,
  hashtagSearch: (prefix: string) => ["notes", "hashtag-search", prefix] as const,
  circles: ["notes", "circles"] as const,
  directCandidates: ["notes", "direct-candidates"] as const,
};

/** Query keys of lists and antennas. Their timelines live with the other timelines (`notesKeys.timeline`). */
export const listKeys = {
  all: ["notes", "lists"] as const,
  mine: ["notes", "lists", "of", "me"] as const,
  of: (userId: string) => ["notes", "lists", "of", userId] as const,
  list: (listId: string) => ["notes", "lists", "one", listId] as const,
  members: (listId: string) => ["notes", "lists", "members", listId] as const,
  followed: ["notes", "lists", "followed"] as const,
  containingMe: ["notes", "lists", "containing-me"] as const,
  antennas: ["notes", "antennas"] as const,
  antenna: (antennaId: string) => ["notes", "antennas", antennaId] as const,
  userSearch: (query: string) => ["notes", "lists", "user-search", query] as const,
};

/** Query keys of clips, under `["notes"]` so live note updates reach a clip's notes. */
export const clipKeys = {
  all: ["notes", "clips"] as const,
  lists: ["notes", "clips", "list"] as const,
  /** `userId` empty = the caller; `noteId` set = the "add to clip" sheet. */
  list: (userId: string, noteId = "") => ["notes", "clips", "list", userId, noteId] as const,
  clip: (clipId: string) => ["notes", "clips", "one", clipId] as const,
  notes: (clipId: string) => ["notes", "clips", "notes", clipId] as const,
};
