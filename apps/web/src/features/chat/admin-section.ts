import { Bot } from "lucide-react";
import i18n from "#/lib/i18n";
import type { AdminSection } from "#/seams/admin";

/** The AI assistant's operator settings in the staff console (admin role). */
export default {
  id: "chat-ai",
  order: 95,
  to: "/admin/chat-ai",
  icon: Bot,
  needs: "admin",
  label: () => i18n.t("chat:aiAdmin.nav"),
} satisfies AdminSection;
