import { Bot } from "lucide-react";
import type { AdminSection } from "#/seams/admin";

/** The AI assistant's operator settings in the staff console (admin role). */
export default {
  id: "chat-ai",
  order: 95,
  to: "/admin/chat-ai",
  icon: Bot,
  needs: "admin",
  label: "chat:aiAdmin.nav",
} satisfies AdminSection;
