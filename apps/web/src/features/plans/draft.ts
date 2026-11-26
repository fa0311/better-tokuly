import type { PlanDraft } from "@tokuly/core/plans";
import { create } from "zustand";

export type { PlanDraft };

/**
 * A plan somebody started from elsewhere — answering "あそぼ!", a day of the
 * calendar, a friend's profile. The create page takes it once and empties it.
 */
const useDraft = create<{ draft: PlanDraft | null }>(() => ({ draft: null }));

export function setPlanDraft(draft: PlanDraft | null): void {
  useDraft.setState({ draft });
}

export function takePlanDraft(): PlanDraft | null {
  const { draft } = useDraft.getState();
  if (draft) useDraft.setState({ draft: null });
  return draft;
}
