/** Query keys of profiles. Every profile hangs under `profiles`, so one invalidation re-reads all. */
export const profileKeys = {
  profiles: ["identity", "profile"] as const,
  profile: (handle: string) => ["identity", "profile", handle] as const,
  byId: (userId: string) => ["identity", "profile", "id", userId] as const,
};
