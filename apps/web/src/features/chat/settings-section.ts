import { Sparkles } from "lucide-react";
import type { SettingsSection } from "#/seams/settings";

/** The row of `/settings` that leads to the AI assistant's settings: provider, own keys. */
export default {
  id: "chat-ai",
  order: 47,
  to: "/settings/chat-ai",
  icon: Sparkles,
  tint: "bg-lime text-[#17122b]",
  title: "chat:aiSettings.rowTitle",
  hint: "chat:aiSettings.rowHint",
} satisfies SettingsSection;
