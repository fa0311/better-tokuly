/** Query keys of moments (いまの瞬間). */
export const momentKeys = {
  all: ["moments"] as const,
  /** The caller's day: the prompt, the window, their own moment, whether friends are unlocked. */
  today: ["moments", "today"] as const,
  /** Friends' moments of their current day (infinite). */
  friends: ["moments", "friends"] as const,
  /** Friends' moments with a place the caller may see, for the map. */
  map: ["moments", "map"] as const,
  moment: (momentId: string) => ["moments", "moment", momentId] as const,
  moments: ["moments", "moment"] as const,
  calendar: (year: number, month: number) => ["moments", "calendar", year, month] as const,
  calendars: ["moments", "calendar"] as const,
  reactions: (momentId: string) => ["moments", "reactions", momentId] as const,
  /** The caller's RealMoji library. */
  realmojis: ["moments", "realmojis"] as const,
  settings: ["moments", "settings"] as const,
};
