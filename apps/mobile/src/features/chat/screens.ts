import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "chat/[id]", options: () => ({ headerShown: false }) },
  {
    name: "new-chat",
    options: () => ({ title: i18n.t("mobile:chat.newChat"), presentation: "modal" }),
  },
] satisfies ScreenEntry[];
