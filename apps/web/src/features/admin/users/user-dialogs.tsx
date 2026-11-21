import { useMutation } from "@tanstack/react-query";
import type { admin } from "@tokuly/api";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { Tabs } from "#/components/ui/tabs";
import { errorMessage } from "#/lib/errors";
import { userAdminClient } from "../api";
import { roleKey } from "../labels";
import { STAFF_ROLES } from "../roles";
import { ActionDialog, NOTE_MAX } from "../ui/action-dialog";
import { DurationSelect } from "../ui/duration";

/** The actions the account page offers; one dialog is open at a time. */
export type UserAction = "warn" | "suspend" | "unsuspend" | "verify" | "roles" | "revoke";

interface DialogProps {
  account: admin.AdminUser;
  open: boolean;
  onOpenChange(open: boolean): void;
  /** Called after the action went through, with the account's new state when the API returned it. */
  onDone(account?: admin.AdminUser): void;
}

const onError = (error: unknown) => toast.error(errorMessage(error));
const handleOf = (account: admin.AdminUser) => account.user?.handle ?? "";

export function WarnDialog({ account, open, onOpenChange, onDone }: DialogProps) {
  const { t } = useTranslation("admin");
  const warn = useMutation({
    mutationFn: (texts: { reason: string; note: string }) => userAdminClient().warnUser({ userId: account.user?.id, ...texts }),
    onSuccess: () => {
      toast.success(t("users.toast.warned"));
      onOpenChange(false);
      onDone();
    },
    onError,
  });
  return (
    <ActionDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("users.warn.title", { handle: handleOf(account) })}
      description={t("users.warn.description")}
      confirmLabel={t("users.warn.confirm")}
      danger
      loading={warn.isPending}
      requireReason
      onConfirm={(texts) => warn.mutate(texts)}
    />
  );
}

export function SuspendDialog({ account, open, onOpenChange, onDone }: DialogProps) {
  const { t } = useTranslation("admin");
  const [permanent, setPermanent] = useState(false);
  const [seconds, setSeconds] = useState(7 * 86_400);
  const suspend = useMutation({
    mutationFn: (texts: { reason: string; note: string }) => userAdminClient().suspendUser({ userId: account.user?.id, ...texts, permanent, suspensionSeconds: permanent ? 0n : BigInt(seconds) }),
    onSuccess: (response) => {
      toast.success(t(permanent ? "users.toast.banned" : "users.toast.suspended"));
      onOpenChange(false);
      onDone(response.user);
    },
    onError,
  });
  return (
    <ActionDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("users.suspend.title", { handle: handleOf(account) })}
      description={t(account.suspension ? "users.suspend.descriptionReplace" : "users.suspend.description")}
      confirmLabel={t(permanent ? "users.suspend.confirmBan" : "users.suspend.confirm")}
      danger
      loading={suspend.isPending}
      requireReason
      onConfirm={(texts) => suspend.mutate(texts)}
    >
      <Tabs<"timed" | "permanent">
        value={permanent ? "permanent" : "timed"}
        onChange={(value) => setPermanent(value === "permanent")}
        items={[
          { value: "timed", label: t("users.suspend.timed") },
          { value: "permanent", label: t("users.suspend.permanent") },
        ]}
      />
      {permanent ? <p className="text-sm text-ink-soft">{t("users.suspend.permanentHint")}</p> : <DurationSelect value={seconds} onChange={setSeconds} />}
    </ActionDialog>
  );
}

export function UnsuspendDialog({ account, open, onOpenChange, onDone }: DialogProps) {
  const { t } = useTranslation("admin");
  const unsuspend = useMutation({
    mutationFn: (note: string) => userAdminClient().unsuspendUser({ userId: account.user?.id, note }),
    onSuccess: (response) => {
      toast.success(t("users.toast.unsuspended"));
      onOpenChange(false);
      onDone(response.user);
    },
    onError,
  });
  return (
    <ActionDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("users.unsuspend.title", { handle: handleOf(account) })}
      description={t("users.unsuspend.description")}
      confirmLabel={t("users.unsuspend.confirm")}
      loading={unsuspend.isPending}
      onConfirm={(texts) => unsuspend.mutate(texts.note)}
    />
  );
}

export function VerifyDialog({ account, open, onOpenChange, onDone }: DialogProps) {
  const { t } = useTranslation("admin");
  const verified = !account.user?.verified;
  const setVerified = useMutation({
    mutationFn: (note: string) => userAdminClient().setUserVerified({ userId: account.user?.id, verified, note }),
    onSuccess: (response) => {
      toast.success(t(verified ? "users.toast.verified" : "users.toast.unverified"));
      onOpenChange(false);
      onDone(response.user);
    },
    onError,
  });
  return (
    <ActionDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t(verified ? "users.verify.grantTitle" : "users.verify.removeTitle", { handle: handleOf(account) })}
      description={t(verified ? "users.verify.grantDescription" : "users.verify.removeDescription")}
      confirmLabel={t(verified ? "users.verify.grant" : "users.verify.remove")}
      danger={!verified}
      loading={setVerified.isPending}
      onConfirm={(texts) => setVerified.mutate(texts.note)}
    />
  );
}

export function RevokeSessionsDialog({ account, open, onOpenChange, onDone }: DialogProps) {
  const { t } = useTranslation("admin");
  const revoke = useMutation({
    mutationFn: (note: string) => userAdminClient().revokeUserSessions({ userId: account.user?.id, note }),
    onSuccess: (response) => {
      toast.success(t("users.toast.revoked", { count: response.revokedCount }));
      onOpenChange(false);
      onDone();
    },
    onError,
  });
  return (
    <ActionDialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("users.revoke.title", { handle: handleOf(account) })}
      description={t("users.revoke.description", { count: account.activeSessionCount })}
      confirmLabel={t("users.revoke.confirm")}
      danger
      loading={revoke.isPending}
      onConfirm={(texts) => revoke.mutate(texts.note)}
    />
  );
}

/** Replaces the account's roles: the switches show the complete new set. */
export function RolesDialog({ account, open, onOpenChange, onDone }: DialogProps) {
  const { t } = useTranslation(["admin", "common"]);
  const [roles, setRoles] = useState<string[]>(account.roles);
  const [note, setNote] = useState("");
  // Every opening starts from the roles the account holds now.
  useEffect(() => {
    if (!open) return;
    setRoles(account.roles);
    setNote("");
  }, [open, account.roles]);

  const save = useMutation({
    mutationFn: () => userAdminClient().setUserRoles({ userId: account.user?.id, roles, note: note.trim() }),
    onSuccess: (response) => {
      toast.success(t("admin:users.toast.rolesChanged"));
      onOpenChange(false);
      onDone(response.user);
    },
    onError,
  });
  const removed = account.roles.filter((role) => !roles.includes(role));
  const changed = removed.length > 0 || roles.some((role) => !account.roles.includes(role));

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("admin:users.roles.title", { handle: handleOf(account) })}
      description={t("admin:users.roles.description")}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={save.isPending}>
            {t("common:actions.cancel")}
          </Button>
          <Button onClick={() => save.mutate()} disabled={!changed} loading={save.isPending}>
            {t("admin:users.roles.confirm")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <div className="-mx-3">
          {STAFF_ROLES.map((role) => (
            <SwitchRow
              key={role}
              label={t(`admin:${roleKey(role)}`)}
              hint={t(`admin:users.roles.hints.${role}`)}
              checked={roles.includes(role)}
              onChange={(checked) => setRoles((current) => (checked ? [...current.filter((held) => held !== role), role] : current.filter((held) => held !== role)))}
            />
          ))}
        </div>
        {removed.length > 0 && <p className="rounded-xl bg-sun px-3 py-2 text-sm font-bold text-[#17122b]">{t("admin:users.roles.removalNotice")}</p>}
        <TextArea label={t("admin:common.note")} hint={t("admin:common.noteHint")} value={note} maxLength={NOTE_MAX} onChange={(event) => setNote(event.target.value)} />
      </div>
    </Dialog>
  );
}
