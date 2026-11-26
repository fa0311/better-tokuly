import type { Api } from "@tokuly/api";
import type { Account } from "#/features/session/account";
import lifecycles from "#/generated/seams/lifecycle";

/**
 * Account lifecycle: what a feature owes an account when it stops being the
 * one in use on this device.
 *
 * A feature that sends anything for the account by itself (its position, a
 * push registration, uploads) default-exports its hooks from
 * `src/features/<area>/lifecycle.ts`:
 *
 *   export default {
 *     beforeLeave: ({ sessionId }) => leavePush(sessionId),
 *     beforeEnd: (session) => session.service(notify.PushService).unregisterDevice({ deviceId: "" }),
 *   } satisfies AccountLifecycle;
 *
 * `features/session/session.ts` runs them; nothing registers at run time, so
 * a hook cannot be missing because a component was not mounted yet.
 */
export interface AccountLifecycle {
  /**
   * The account in use is about to leave: a sign-out, a switch that went
   * through, or another account being added beside it. Its session still
   * works, and `account.api` is the object that speaks for it — the active
   * one may be someone else's by the time an await returns. Stop sending for
   * it and tell the server. The hooks of all features run together and each
   * gets `LEAVE_TIMEOUT_MS`; the account leaves when they are done, failed
   * or late, so what must reach the server is remembered as owed first and
   * retried when that account is back (or `asParked`).
   */
  beforeLeave?(account: Account): Promise<void> | void;
  /**
   * A session that left this device (signed out offline, or found from a
   * previous install) is resumed once more, right before the server ends it:
   * take back what it left there. `session` speaks for it and for nobody
   * else. Throwing keeps the session aside, and all of this runs again at
   * the next start.
   */
  beforeEnd?(session: Api): Promise<unknown>;
  /**
   * Secure-storage entries the feature keeps for this device's accounts
   * (an unanswered gift). The first launch of an install forgets them with
   * the accounts: the iOS keychain outlives an uninstall.
   */
  secureKeys?: readonly string[];
}

const LIFECYCLES: AccountLifecycle[] = lifecycles;

export const LEAVE_TIMEOUT_MS = 3000;

/** Every feature's `secureKeys`, for the first-launch wipe. */
export const FEATURE_SECURE_KEYS: readonly string[] = LIFECYCLES.flatMap(
  ({ secureKeys }) => secureKeys ?? [],
);

/** Every feature gets its moment before `account` leaves; one that fails or hangs is left behind. */
export async function runBeforeLeave(account: Account, lifecycles = LIFECYCLES): Promise<void> {
  const run = (hook: NonNullable<AccountLifecycle["beforeLeave"]>) =>
    new Promise<void>((resolve) => {
      const timer = setTimeout(resolve, LEAVE_TIMEOUT_MS);
      const done = () => {
        clearTimeout(timer);
        resolve();
      };
      try {
        Promise.resolve(hook(account)).then(done, done);
      } catch {
        done();
      }
    });
  await Promise.all(
    lifecycles.flatMap(({ beforeLeave }) => (beforeLeave ? [run(beforeLeave)] : [])),
  );
}

/** What the features take back as a departed session; the first failure stops it from being ended. */
export async function runBeforeEnd(session: Api, lifecycles = LIFECYCLES): Promise<void> {
  for (const { beforeEnd } of lifecycles) await beforeEnd?.(session);
}
