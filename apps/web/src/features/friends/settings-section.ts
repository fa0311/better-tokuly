import { Lock } from "lucide-react";
import i18n from "#/lib/i18n";
import type { SettingsSection } from "#/seams/settings";
import { useFollowRequestCount } from "./follow-data";

/** The row of `/settings` that leads to the follow settings: 鍵アカウント and waiting requests. */
export default {
  id: "follows",
  order: 42,
  to: "/settings/follows",
  icon: Lock,
  tint: "bg-mint text-[#17122b]",
  title: () => i18n.t("follows:settingsRow.title"),
  hint: () => i18n.t("follows:settingsRow.hint"),
  useAttention: () => {
    const waiting = useFollowRequestCount();
    return waiting > 0 ? i18n.t("follows:settingsRow.waiting", { count: waiting }) : undefined;
  },
} satisfies SettingsSection;
