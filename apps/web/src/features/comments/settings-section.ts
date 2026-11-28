import { MessageSquareText } from "lucide-react";
import { useTranslation } from "#/lib/use-translation";
import type { SettingsSection } from "#/seams/settings";
import { useHeldCount } from "./api";

/** The row of `/settings` that leads to comment moderation: held comments, rules, blocked words. */
export default {
  id: "comments",
  order: 45,
  to: "/settings/comments",
  icon: MessageSquareText,
  tint: "bg-sky text-[#17122b]",
  title: "comments:settingsRow.title",
  hint: "comments:settingsRow.hint",
  useAttention: () => {
    const { t } = useTranslation("comments");
    const held = useHeldCount().data ?? 0;
    return held > 0 ? t("settingsRow.held", { count: held }) : undefined;
  },
} satisfies SettingsSection;
