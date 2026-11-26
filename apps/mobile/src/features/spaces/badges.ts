import type { BadgeSources } from "#/seams/badges";
import { useHangouts } from "./data";

/** `spaces`: invitations to hangouts that wait for an answer. */
export default {
  spaces: () => useHangouts().data?.invitations.length ?? 0,
} satisfies BadgeSources;
