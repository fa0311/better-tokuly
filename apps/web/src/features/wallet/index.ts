/**
 * What the wallet shares with other features. Coins are spent in other
 * places (the stamp store, live gifts); they show the balance with these and
 * send people who are short to `/wallet/buy`.
 */
export { BalancePill } from "./balance-pill";
export { type BalanceState, useBalance } from "./use-balance";
