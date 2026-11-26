import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "ask/index", options: () => ({ title: i18n.t("ask:title") }) },
  { name: "ask/inbox", options: () => ({ title: i18n.t("ask:inbox.title") }) },
  { name: "ask/sent", options: () => ({ title: i18n.t("ask:sent.title") }) },
  { name: "ask/u/[handle]", options: () => ({ title: i18n.t("ask:title") }) },
  { name: "ask/q/[id]", options: () => ({ title: i18n.t("ask:question.title") }) },
  {
    name: "ask/answer/[id]",
    options: () => ({ title: i18n.t("ask:answer.title"), presentation: "modal" }),
  },
] satisfies ScreenEntry[];
