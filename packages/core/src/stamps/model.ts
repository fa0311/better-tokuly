import { stamp } from "@tokuly/api";

/** The i18n key (in the `stamps` namespace) naming a pack status. */
export function statusKey(status: stamp.PackStatus): string {
  switch (status) {
    case stamp.PackStatus.IN_REVIEW:
      return "status.inReview";
    case stamp.PackStatus.PUBLISHED:
      return "status.published";
    case stamp.PackStatus.REJECTED:
      return "status.rejected";
    case stamp.PackStatus.REMOVED:
      return "status.removed";
    default:
      return "status.draft";
  }
}

/** The i18n key (in the `stamps` namespace) naming where a stamp was sent. */
export function contextKey(context: stamp.UsageContext): string {
  switch (context) {
    case stamp.UsageContext.CHAT:
      return "context.chat";
    case stamp.UsageContext.LIVE_CHAT:
      return "context.liveChat";
    case stamp.UsageContext.COMMENT:
      return "context.comment";
    case stamp.UsageContext.NOTE:
      return "context.note";
    case stamp.UsageContext.MAP:
      return "context.map";
    default:
      return "context.unknown";
  }
}
