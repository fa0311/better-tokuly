import { MessageSquareText } from "lucide-react";
import i18n from "#/lib/i18n";
import type { SettingsSection } from "#/seams/settings";
import { useHeldCount } from "./api";

/** The row of `/settings` that leads to comment moderation: held comments, rules, blocked words. */
export default {
  id: "comments",
  order: 45,
  to: "/settings/comments",
  icon: MessageSquareText,
  tint: "bg-sky text-[#17122b]",
  title: () => i18n.t("comments:settingsRow.title"),
  hint: () => i18n.t("comments:settingsRow.hint"),
  useAttention: () => {
    const held = useHeldCount().data ?? 0;
    return held > 0 ? i18n.t("comments:settingsRow.held", { count: held }) : undefined;
  },
} satisfies SettingsSection;
