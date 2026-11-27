import { useEffect } from "react";
import { toggleQuickSearch } from "./palette";
import { QuickSearch } from "./quick-search";
import { SEARCH_BOX_ATTRIBUTE } from "./search-box";

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
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.repeat) return;
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
  return <QuickSearch />;
}
