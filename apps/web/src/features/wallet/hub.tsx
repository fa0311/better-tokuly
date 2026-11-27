import { Wallet } from "lucide-react";
import type { HubTile } from "#/seams/hub";

/** The way in to the wallet from the hub. */
export default [
  {
    id: "wallet",
    section: "me",
    order: 20,
    to: "/wallet",
    label: "common:nav.wallet",
    icon: Wallet,
    tint: "bg-lime",
    keywords: ["wallet", "coins", "ウォレット", "コイン"],
  },
] satisfies HubTile[];
