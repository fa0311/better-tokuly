import { wallet } from "@tokuly/api";
import { api } from "#/lib/api";

/**
 * The wallet's clients. Keys, query options and cache edits are
 * `@tokuly/core/wallet`; this file wires them to the web's API client.
 */
export const walletClient = () => api().service(wallet.WalletService);
export const earningsClient = () => api().service(wallet.EarningsService);
export const walletAdminClient = () => api().service(wallet.WalletAdminService);

/** Rows per page of the wallet's lists. */
export const PAGE_SIZE = 30;
