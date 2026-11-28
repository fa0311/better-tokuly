import { Lock } from "lucide-react";
import { useTranslation } from "#/lib/use-translation";
import type { SettingsSection } from "#/seams/settings";
import { useFollowRequestCount } from "./follow-data";

/** The row of `/settings` that leads to the follow settings: 鍵アカウント and waiting requests. */
export default {
  id: "follows",
  order: 42,
  to: "/settings/follows",
  icon: Lock,
  tint: "bg-mint text-[#17122b]",
  title: "follows:settingsRow.title",
  hint: "follows:settingsRow.hint",
  useAttention: () => {
    const { t } = useTranslation("follows");
    const waiting = useFollowRequestCount();
    return waiting > 0 ? t("settingsRow.waiting", { count: waiting }) : undefined;
  },
} satisfies SettingsSection;
