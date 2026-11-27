import { create } from "zustand";

/** Whether the quick search is open, apart from the palette itself so a button can open it cheaply. */
export const usePalette = create<{ open: boolean; query: string }>(() => ({
  open: false,
  query: "",
}));

/** Opens the quick search over whatever page is showing, optionally with something typed already. */
export function openQuickSearch(query = ""): void {
  usePalette.setState({ open: true, query });
}

export function closeQuickSearch(): void {
  usePalette.setState({ open: false });
}

export function toggleQuickSearch(): void {
  usePalette.setState((state) => ({ open: !state.open, query: "" }));
}
