import type { Subject } from "./model";

/** Query keys of the engagement domain. */
export const engagementKeys = {
  all: ["engagement"] as const,
  subjects: ["engagement", "subject"] as const,
  subject: (subject: Subject) => ["engagement", "subject", subject.type, subject.id] as const,
  reactors: (subject: Subject, emoji: string) =>
    ["engagement", "reactors", subject.type, subject.id, emoji] as const,
  myReactions: ["engagement", "my-reactions"] as const,
  reactionSettings: ["engagement", "reaction-settings"] as const,
  collections: ["engagement", "collections"] as const,
  collectionsFor: (subject: Subject) =>
    ["engagement", "collections", "for", subject.type, subject.id] as const,
  bookmarks: ["engagement", "bookmarks"] as const,
  bookmarkList: (collectionId: string) => ["engagement", "bookmarks", collectionId] as const,
  reports: ["engagement", "reports"] as const,
  shareStats: (subject: Subject) =>
    ["engagement", "share-stats", subject.type, subject.id] as const,
};
