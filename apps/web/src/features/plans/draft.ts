import type { plans } from "@tokuly/api";
import { create } from "zustand";

/** What a new plan starts from: the fields of a `CreatePlanRequest` that are known already. */
export type PlanDraft = Partial<
  Pick<plans.CreatePlanRequest, "title" | "emoji" | "description" | "time" | "place" | "inviteeIds">
>;

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
