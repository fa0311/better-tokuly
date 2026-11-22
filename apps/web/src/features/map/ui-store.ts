import { create } from "zustand";

/** What the sheet (phones) / side panel (wide screens) is showing. */
export type Panel =
  | { kind: "friends" }
  | { kind: "friend"; userId: string }
  | { kind: "gathering"; id: string }
  | { kind: "me" }
  | { kind: "ghost" }
  | { kind: "trips" }
  | { kind: "safety" }
  | { kind: "shares" }
  | { kind: "story"; userId: string };

interface MapUiStore {
  /** Panels opened on top of each other; "back" pops one. */
  stack: Panel[];
  /** Pixels of the map covered by floating chrome, per reporting element. */
  insets: Record<string, { side: "top" | "bottom" | "left"; size: number }>;
  /** Replaces whatever is open. */
  open(panel: Panel): void;
  /** Opens on top of the current panel, so "back" returns to it. */
  push(panel: Panel): void;
  back(): void;
  close(): void;
  setInset(id: string, side: "top" | "bottom" | "left", size: number | null): void;
}

export const useMapUi = create<MapUiStore>((set) => ({
  stack: [],
  insets: {},
  open: (panel) => set({ stack: [panel] }),
  push: (panel) => set((state) => ({ stack: [...state.stack, panel] })),
  back: () => set((state) => ({ stack: state.stack.slice(0, -1) })),
  close: () => set({ stack: [] }),
  setInset: (id, side, size) =>
    set((state) => {
      const current = state.insets[id];
      if (size === null) {
        if (!current) return state;
        const { [id]: _gone, ...insets } = state.insets;
        return { insets };
      }
      if (current && current.side === side && Math.abs(current.size - size) < 2) return state;
      return { insets: { ...state.insets, [id]: { side, size } } };
    }),
}));

/** The friend whose sheet is open, if any (their pin is highlighted). */
export function useFocusedUserId(): string | null {
  return useMapUi((state) => {
    const top = state.stack.at(-1);
    return top?.kind === "friend" || top?.kind === "story" ? top.userId : null;
  });
}
