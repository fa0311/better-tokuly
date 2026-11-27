import type { wallet } from "@tokuly/api";

/**
 * Query keys of the wallet. Everything the signed-in user sees about their
 * own coins lives under `["wallet", …]`; the staff tools under
 * `["wallet", "admin", …]`.
 */
export const walletKeys = {
  all: ["wallet"] as const,
  balance: ["wallet", "balance"] as const,
  ledgers: ["wallet", "ledger"] as const,
  ledger: (account: wallet.LedgerAccount) => ["wallet", "ledger", account] as const,
  packages: ["wallet", "packages"] as const,
  purchases: ["wallet", "purchases"] as const,
  purchase: (purchaseId: string) => ["wallet", "purchase", purchaseId] as const,
  dailyBonus: ["wallet", "dailyBonus"] as const,
  earnings: ["wallet", "earnings"] as const,
  earningsSummary: (range: string) => ["wallet", "earnings", range] as const,
  payouts: ["wallet", "payouts"] as const,
  payout: (payoutId: string) => ["wallet", "payouts", "one", payoutId] as const,
  admin: {
    all: ["wallet", "admin"] as const,
    queues: ["wallet", "admin", "queue"] as const,
    queue: (status: wallet.PayoutStatus) => ["wallet", "admin", "queue", status] as const,
    pendingCount: ["wallet", "admin", "queue", "pendingCount"] as const,
    payout: (payoutId: string) => ["wallet", "admin", "payout", payoutId] as const,
    userSearch: (query: string) => ["wallet", "admin", "userSearch", query] as const,
    userWallet: (userId: string) => ["wallet", "admin", "user", userId] as const,
    userLedger: (userId: string, account: wallet.LedgerAccount) =>
      ["wallet", "admin", "userLedger", userId, account] as const,
    adjustments: (userId: string) => ["wallet", "admin", "adjustments", userId] as const,
    allAdjustments: ["wallet", "admin", "adjustments"] as const,
    packages: ["wallet", "admin", "packages"] as const,
  },
};
