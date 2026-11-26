import i18n from "#/lib/i18n";
import type { ScreenEntry } from "#/seams/screens";

export default [
  { name: "notes/[id]", options: () => ({ title: i18n.t("notes:thread.title") }) },
  {
    name: "notes/compose",
    // The title follows what is written (a note, a reply, a quote).
    options: () => ({ title: i18n.t("notes:compose.title.new"), presentation: "modal" }),
  },
  { name: "notes/drafts", options: () => ({ title: i18n.t("notes:drafts.title") }) },
  { name: "notes/clips/index", options: () => ({ title: i18n.t("clips:mine") }) },
  { name: "notes/clips/[id]", options: () => ({ title: i18n.t("clips:title") }) },
  { name: "notes/tag/[tag]", options: () => ({ title: i18n.t("notes:tag.title") }) },
  { name: "notes/list/[id]", options: () => ({ title: i18n.t("notelists:lists.title") }) },
  { name: "notes/antenna/[id]", options: () => ({ title: i18n.t("notelists:antennas.title") }) },
  { name: "notes/user/[id]", options: () => ({ title: i18n.t("notes:profile.title") }) },
] satisfies ScreenEntry[];
