import { Wallet } from "lucide-react";
import type { AdminSection } from "#/seams/admin";

/** The entry of this area's review page in the staff console. */
export default {
  id: "wallet",
  order: 80,
  to: "/admin/wallet",
  icon: Wallet,
  needs: "staff",
  label: "admin:nav.wallet",
} satisfies AdminSection;
