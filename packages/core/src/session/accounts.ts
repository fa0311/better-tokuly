import type { identity } from "@tokuly/api";
import type { ClientSource } from "../query";

/** Query keys of the accounts signed in on this device. */
export const accountKeys = {
  deviceAccounts: ["identity", "device-accounts"] as const,
};

/**
 * The accounts on this device (a main one, a 裏垢, a creator account). Each
 * keeps its own sign-in; how the app then starts over as another one (a page
 * load, a fresh navigation stack) is the app's.
 */
export function deviceAccountsQuery(client: ClientSource<typeof identity.AuthService>) {
  return {
    queryKey: accountKeys.deviceAccounts,
    queryFn: () => client().listDeviceAccounts({}),
  };
}

/** Whether another account may still be added on this device. */
export function canAddAccount(list: identity.ListDeviceAccountsResponse | undefined): boolean {
  return list ? list.accounts.length < list.limit : true;
}
