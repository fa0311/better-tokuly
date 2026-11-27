export type BoardSort = "hot" | "new" | "top";

const BOARD_WINDOWS = ["day", "week", "month", "year", "all"] as const;
export type BoardWindow = (typeof BOARD_WINDOWS)[number];

export interface BoardSearch {
  channel?: string;
  sort?: BoardSort;
  window?: BoardWindow;
  tag?: string;
}

export function parseBoardSearch(search: Record<string, unknown>): BoardSearch {
  const text = (value: unknown) => (typeof value === "string" && value ? value : undefined);
  const sort = text(search.sort);
  const window = text(search.window);
  return {
    channel: text(search.channel),
    sort: sort === "new" || sort === "top" ? sort : undefined,
    window: BOARD_WINDOWS.find((each) => each === window && each !== "all"),
    tag: text(search.tag),
  };
}
