/** Query keys of stories (記憶). They live apart from notes: nothing here holds a note. */
export const storyKeys = {
  all: ["stories"] as const,
  tray: ["stories", "tray"] as const,
  user: (userId: string) => ["stories", "user", userId] as const,
  story: (storyId: string) => ["stories", "story", storyId] as const,
  viewers: (storyId: string) => ["stories", "viewers", storyId] as const,
  calendar: (year: number, month: number) => ["stories", "calendar", year, month] as const,
  calendars: ["stories", "calendar"] as const,
  archive: (date: string) => ["stories", "archive", date] as const,
  archives: ["stories", "archive"] as const,
  highlights: (userId: string) => ["stories", "highlights", userId] as const,
  highlight: (highlightId: string) => ["stories", "highlight", highlightId] as const,
};
