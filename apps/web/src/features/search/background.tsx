import { lazy, Suspense, useEffect, useState } from "react";
import { SEARCH_BOX_ATTRIBUTE, toggleQuickSearch, usePalette } from "./palette";

const QuickSearch = lazy(() =>
  import("./quick-search").then((module) => ({ default: module.QuickSearch })),
);

/** Whether a key press belongs to something that is being typed into. */
function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.isContentEditable ||
    target.matches("input, textarea, select, [role='textbox'], [role='combobox']")
  );
}

/** Another dialog is on screen: the shortcuts stay out of its way. */
function dialogOpen(): boolean {
  return document.querySelector("[role='dialog'], [role='alertdialog']") !== null;
}

/**
 * Search from anywhere: Cmd/Ctrl+K, or "/" when nothing is being typed,
 * opens the quick search. On a page with a search field of its own (`/search`)
 * the shortcuts put the cursor there instead.
 */
export default function SearchBackground() {
  // The palette loads the first time it opens, and stays mounted so it can animate out.
  const open = usePalette((state) => state.open);
  const [opened, setOpened] = useState(false);
  if (open && !opened) setOpened(true);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.repeat) return;
      // The first opening waits for the palette's code: what is typed meanwhile is its query.
      const loading =
        usePalette.getState().open && document.querySelector("[data-quick-search]") === null;
      if (loading && event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey) {
        event.preventDefault();
        usePalette.setState((state) => ({ query: state.query + event.key }));
        return;
      }
      const commandK =
        event.key.toLowerCase() === "k" &&
        (event.metaKey || event.ctrlKey) &&
        !event.altKey &&
        !event.shiftKey;
      const slash =
        event.key === "/" &&
        !event.metaKey &&
        !event.ctrlKey &&
        !event.altKey &&
        !isTyping(event.target);
      if (!commandK && !slash) return;
      const ownField = document.querySelector<HTMLInputElement>(`[${SEARCH_BOX_ATTRIBUTE}]`);
      const paletteOpen = document.querySelector("[data-quick-search]") !== null;
      if (!paletteOpen && dialogOpen()) return;
      event.preventDefault();
      if (ownField && !paletteOpen) {
        ownField.focus();
        ownField.select();
      } else {
        // Cmd/Ctrl+K also closes the palette again ("/" cannot get here while it is open: its field has the focus).
        toggleQuickSearch();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
  return (
    opened && (
      <Suspense fallback={null}>
        <QuickSearch />
      </Suspense>
    )
  );
}
