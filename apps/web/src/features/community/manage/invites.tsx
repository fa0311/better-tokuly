import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { community } from "@tokuly/api";
import { communityKeys, inviteHref } from "@tokuly/core/community";
import { Copy, Plus, QrCode, Trash2 } from "lucide-react";
import QRCode from "qrcode";
import { useEffect, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { SelectField } from "#/components/ui/select";
import { copyText } from "#/features/engagement";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { communityClient, NS, useInvites } from "../api";
import { Notice, useDurationLabel, useWhen } from "../parts";

const EXPIRIES = [0, 3600, 86_400, 7 * 86_400, 30 * 86_400] as const;
const USES = [0, 1, 5, 10, 50, 100] as const;

/** The link this app opens an invite at (the QR code carries the same). */
const linkOf = (invite: community.Invite) => `${window.location.origin}${inviteHref(invite.code)}`;

/** Invites: one token usable as a code, a link and a QR code, with an optional expiry and use limit. */
export function InvitesSection({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const when = useWhen();
  const queryClient = useQueryClient();
  const invites = useInvites(c.id);
  const [creating, setCreating] = useState(false);
  const [showing, setShowing] = useState<community.Invite>();
  const [revoking, setRevoking] = useState<community.Invite>();
  const revoke = useMutation({
    mutationFn: (inviteId: string) => communityClient().revokeInvite({ inviteId }),
    onSuccess: () => {
      setRevoking(undefined);
      void queryClient.invalidateQueries({ queryKey: communityKeys.invites(c.id) });
      toast(t("invite.revoked"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  const copy = async (invite: community.Invite) => {
    if (await copyText(linkOf(invite))) toast.success(t("share.copied"));
    else toast.error(t("share.copyFailed"));
  };
  return (
    <div className="flex flex-col gap-3">
      <Notice>{t("invite.hint")}</Notice>
      <Button
        className="self-start"
        icon={<Plus className="size-4" />}
        onClick={() => setCreating(true)}
      >
        {t("invite.create")}
      </Button>
      {invites.isPending ? (
        <Skeleton className="h-24" />
      ) : invites.items?.length === 0 ? (
        <EmptyState emoji="💌" title={t("invite.empty")} body={t("invite.emptyBody")} />
      ) : (
        <>
          {invites.items?.map((invite) => (
            <article key={invite.id} className="rounded-card bg-surface p-4">
              <div className="flex items-center gap-2">
                <code className="min-w-0 flex-1 truncate font-mono text-lg font-black tracking-wider">
                  {invite.code}
                </code>
                <Button
                  variant="secondary"
                  size="icon-sm"
                  aria-label={t("invite.copy", { code: invite.code })}
                  onClick={() => void copy(invite)}
                >
                  <Copy className="size-4" />
                </Button>
                <Button
                  variant="secondary"
                  size="icon-sm"
                  aria-label={t("invite.qr", { code: invite.code })}
                  onClick={() => setShowing(invite)}
                >
                  <QrCode className="size-4" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t("invite.revoke", { code: invite.code })}
                  onClick={() => setRevoking(invite)}
                >
                  <Trash2 className="size-4 text-danger" />
                </Button>
              </div>
              {invite.note && <p className="mt-1 break-words text-sm">{invite.note}</p>}
              <div className="mt-2 flex flex-wrap gap-1.5">
                <Chip>
                  {invite.maxUses > 0
                    ? t("invite.usesOf", { count: invite.useCount, max: invite.maxUses })
                    : t("invite.uses", { count: invite.useCount })}
                </Chip>
                <Chip>
                  {invite.expiresAt
                    ? t("invite.expires", { time: when(Number(invite.expiresAt.seconds) * 1000) })
                    : t("invite.noExpiry")}
                </Chip>
                {invite.creator && <Chip>@{invite.creator.handle}</Chip>}
              </div>
            </article>
          ))}
          <LoadMore query={invites} />
        </>
      )}
      {creating && (
        <CreateInviteDialog
          communityId={c.id}
          onClose={() => setCreating(false)}
          onCreated={setShowing}
        />
      )}
      <InviteQrDialog invite={showing} name={c.name} onClose={() => setShowing(undefined)} />
      <ConfirmDialog
        open={Boolean(revoking)}
        onOpenChange={(open) => !open && setRevoking(undefined)}
        title={t("invite.revokeTitle")}
        description={t("invite.revokeBody")}
        confirmLabel={t("invite.revokeAction")}
        danger
        loading={revoke.isPending}
        onConfirm={() => revoking && revoke.mutate(revoking.id)}
      />
    </div>
  );
}

function CreateInviteDialog({
  communityId,
  onClose,
  onCreated,
}: {
  communityId: string;
  onClose(): void;
  onCreated(invite: community.Invite): void;
}) {
  const { t } = useTranslation(NS);
  const durationLabel = useDurationLabel();
  const queryClient = useQueryClient();
  const [expiry, setExpiry] = useState("604800");
  const [uses, setUses] = useState("0");
  const [note, setNote] = useState("");
  const create = useMutation({
    mutationFn: () =>
      communityClient().createInvite({
        communityId,
        expiresInSeconds: Number(expiry),
        maxUses: Number(uses),
        note: note.trim(),
      }),
    onSuccess: ({ invite }) => {
      void queryClient.invalidateQueries({ queryKey: communityKeys.invites(communityId) });
      onClose();
      if (invite) onCreated(invite);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("invite.create")}
      footer={
        <Button loading={create.isPending} onClick={() => create.mutate()}>
          {t("invite.createAction")}
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        <SelectField
          label={t("invite.expiry")}
          value={expiry}
          onChange={setExpiry}
          options={EXPIRIES.map((value) => ({
            value: String(value),
            label: value === 0 ? t("invite.noExpiry") : durationLabel(value),
          }))}
        />
        <SelectField
          label={t("invite.maxUses")}
          value={uses}
          onChange={setUses}
          options={USES.map((value) => ({
            value: String(value),
            label: value === 0 ? t("invite.unlimited") : t("invite.people", { count: value }),
          }))}
        />
        <TextField
          label={t("invite.note")}
          hint={t("invite.noteHint")}
          value={note}
          maxLength={100}
          onChange={(event) => setNote(event.target.value)}
        />
      </div>
    </Dialog>
  );
}

/** An invite to show or send: the QR code, the code, the link. */
function InviteQrDialog({
  invite,
  name,
  onClose,
}: {
  invite: community.Invite | undefined;
  name: string;
  onClose(): void;
}) {
  const { t } = useTranslation(NS);
  const [qr, setQr] = useState("");
  const link = invite ? linkOf(invite) : "";
  useEffect(() => {
    if (!link) return;
    let cancelled = false;
    setQr("");
    QRCode.toDataURL(link, { margin: 1, width: 480, color: { dark: "#17122b", light: "#ffffff" } })
      .then((dataUrl) => {
        if (!cancelled) setQr(dataUrl);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [link]);
  return (
    <Dialog
      open={Boolean(invite)}
      onOpenChange={(open) => !open && onClose()}
      title={t("invite.qrTitle", { name })}
      description={t("invite.qrBody")}
    >
      {invite && (
        <div className="flex flex-col items-center gap-4">
          <div className="sticker grid size-56 place-items-center rounded-card bg-white p-3">
            {qr && <img src={qr} alt={t("invite.qrAlt")} className="size-full" />}
          </div>
          <code className="font-mono text-2xl font-black tracking-[0.15em]">{invite.code}</code>
          <p className="max-w-full break-all text-center text-sm text-ink-soft">{link}</p>
          <Button
            icon={<Copy className="size-4" />}
            onClick={async () => {
              if (await copyText(link)) toast.success(t("share.copied"));
              else toast.error(t("share.copyFailed"));
            }}
          >
            {t("invite.copyLink")}
          </Button>
        </div>
      )}
    </Dialog>
  );
}
