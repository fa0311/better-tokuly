import { Wallet } from "lucide-react";
import i18n from "#/lib/i18n";
import type { AdminSection } from "#/seams/admin";

/** The entry of this area's review page in the staff console. */
export default {
  id: "wallet",
  order: 80,
  to: "/admin/wallet",
  icon: Wallet,
  needs: "staff",
  label: () => i18n.t("admin:nav.wallet"),
} satisfies AdminSection;
