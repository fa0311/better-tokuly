import { admin } from "@tokuly/api";
import { useTranslation } from "react-i18next";
import { formatDateTime } from "../format";
import { accountStateKey, roleKey } from "../labels";
import { type Tone, ToneChip } from "../ui/tone";

const STATE_TONES: Partial<Record<admin.AccountState, Tone>> = {
  [admin.AccountState.ACTIVE]: "good",
  [admin.AccountState.SUSPENDED]: "danger",
  [admin.AccountState.DELETION_SCHEDULED]: "warn",
};

/** Whether the account is usable, with the end of a timed suspension when there is one. */
export function AccountStateChip({ account }: { account: admin.AdminUser }) {
  const { t } = useTranslation("admin");
  const suspension =
    account.state === admin.AccountState.SUSPENDED ? account.suspension : undefined;
  return (
    <span className="inline-flex flex-wrap items-center gap-x-2 gap-y-1">
      <ToneChip tone={STATE_TONES[account.state] ?? "neutral"}>
        {suspension?.permanent ? t("users.banned") : t(accountStateKey(account.state))}
      </ToneChip>
      {suspension && !suspension.permanent && suspension.expiresAt && (
        <span className="text-xs whitespace-nowrap text-ink-soft">
          {t("users.until", { time: formatDateTime(suspension.expiresAt) })}
        </span>
      )}
    </span>
  );
}

export function RoleChips({ roles, emptyLabel }: { roles: string[]; emptyLabel?: string }) {
  const { t } = useTranslation("admin");
  if (roles.length === 0) return <span className="text-ink-faint">{emptyLabel ?? "—"}</span>;
  return (
    <span className="inline-flex flex-wrap gap-1">
      {roles.map((role) => (
        <ToneChip key={role} tone={role === "admin" ? "info" : "brand"}>
          {t(roleKey(role))}
        </ToneChip>
      ))}
    </span>
  );
}
