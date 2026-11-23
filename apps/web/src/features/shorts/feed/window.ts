export interface PageInWindow {
  param: string;
  length: number;
}

/** The loaded pages of a feed and the position of the first one's first entry in the whole feed. */
export interface FeedWindow {
  pages: PageInWindow[];
  offset: number;
}

/**
 * The window after the loaded pages changed: pages let go at the front move
 * the offset on by what they held, pages read again at the front move it
 * back by what they hold. Anything else is a feed that started over.
 */
export function moveWindow(current: FeedWindow, pages: PageInWindow[]): FeedWindow {
  const same =
    current.pages.length === pages.length &&
    current.pages.every(
      (page, index) => page.param === pages[index]?.param && page.length === pages[index]?.length,
    );
  if (same) return current;
  const first = pages[0]?.param;
  if (first === undefined) return { pages, offset: 0 };
  if (first === current.pages[0]?.param) return { pages, offset: current.offset };
  const dropped = current.pages.findIndex((page) => page.param === first);
  if (dropped > 0) {
    const gone = current.pages.slice(0, dropped).reduce((sum, page) => sum + page.length, 0);
    return { pages, offset: current.offset + gone };
  }
  const added = pages.findIndex((page) => page.param === current.pages[0]?.param);
  if (added > 0) {
    const back = pages.slice(0, added).reduce((sum, page) => sum + page.length, 0);
    return { pages, offset: Math.max(0, current.offset - back) };
  }
  return { pages, offset: 0 };
}
