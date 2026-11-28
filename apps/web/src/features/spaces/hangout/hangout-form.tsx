import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import type { common, spaces } from "@tokuly/api";
import { rememberHangout, spacesKeys } from "@tokuly/core/spaces";
import { Plus, X } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { SwitchRow } from "#/components/ui/switch";
import { EmojiPicker } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { hangoutClient } from "../api";
import { FriendPicker } from "../friend-picker";
import { PersonName } from "../parts";

const DEFAULT_EMOJI = "🛋️";

/** Creates a hangout, or changes its name, emoji and who may invite (the owner's settings). */
export function HangoutForm({
  open,
  onOpenChange,
  hangout,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  /** The hangout being changed; without it the form creates one. */
  hangout?: spaces.Hangout;
}) {
  const { t } = useTranslation("spaces");
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t(hangout ? "hangoutForm.editTitle" : "hangoutForm.createTitle")}
      description={hangout ? undefined : t("hangoutForm.body")}
    >
      {open && <Fields hangout={hangout} onDone={() => onOpenChange(false)} />}
    </Dialog>
  );
}

function Fields({ hangout, onDone }: { hangout?: spaces.Hangout; onDone(): void }) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [name, setName] = useState(hangout?.name ?? "");
  const [emoji, setEmoji] = useState(hangout?.emoji || DEFAULT_EMOJI);
  const [membersCanInvite, setMembersCanInvite] = useState(hangout?.membersCanInvite ?? true);
  const [invitees, setInvitees] = useState<common.UserSummary[]>([]);
  const [picking, setPicking] = useState(false);

  const save = useMutation({
    mutationFn: async () => {
      const fields = { name: name.trim(), emoji, membersCanInvite };
      if (hangout)
        return (await hangoutClient().updateHangout({ hangoutId: hangout.id, ...fields })).hangout;
      return (
        await hangoutClient().createHangout({
          ...fields,
          inviteeIds: invitees.map((user) => user.id),
        })
      ).hangout;
    },
    onSuccess: (saved) => {
      rememberHangout(queryClient, saved);
      void queryClient.invalidateQueries({ queryKey: spacesKeys.hangouts });
      onDone();
      if (hangout) return void toast.success(t("toast.saved"));
      toast.success(t("toast.hangoutCreated"));
      if (saved)
        void navigate({ to: "/spaces/hangouts/$hangoutId", params: { hangoutId: saved.id } });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "spaces" })),
  });

  return (
    <form
      className="flex flex-col gap-5"
      onSubmit={(event) => {
        event.preventDefault();
        if (name.trim() && !save.isPending) save.mutate();
      }}
    >
      <div className="flex items-end gap-3">
        <EmojiPicker
          unicodeOnly
          side="bottom"
          onPick={setEmoji}
          trigger={
            <button
              type="button"
              aria-label={t("hangoutForm.emoji")}
              className="pressable sticker grid size-12 shrink-0 place-items-center rounded-2xl bg-surface text-2xl leading-none"
            >
              {emoji}
            </button>
          }
        />
        <TextField
          className="min-w-0 flex-1"
          label={t("hangoutForm.name")}
          placeholder={t("hangoutForm.namePlaceholder")}
          value={name}
          maxLength={40}
          onChange={(event) => setName(event.target.value)}
          autoFocus={!hangout}
          required
        />
      </div>
      <SwitchRow
        label={t("hangoutForm.membersCanInvite")}
        hint={t("hangoutForm.membersCanInviteHint")}
        checked={membersCanInvite}
        onChange={setMembersCanInvite}
      />
      {!hangout && (
        <div className="flex flex-col gap-1.5">
          <p className="px-1 text-sm font-bold text-ink-soft">{t("hangoutForm.invitees")}</p>
          <div className="flex flex-wrap items-center gap-2">
            {invitees.map((user) => (
              <span
                key={user.id}
                className="inline-flex h-10 max-w-full items-center gap-2 rounded-full border-2 border-line bg-surface pr-1 pl-1.5"
              >
                <Avatar user={user} size="xs" />
                <span className="truncate text-sm font-bold">
                  <PersonName user={user} />
                </span>
                <button
                  type="button"
                  aria-label={t("picker.remove", { name: user.displayName || user.handle })}
                  onClick={() => setInvitees(invitees.filter((other) => other.id !== user.id))}
                  className="grid size-7 shrink-0 place-items-center rounded-full text-ink-faint hover:bg-surface-2 hover:text-ink"
                >
                  <X className="size-4" />
                </button>
              </span>
            ))}
            <button
              type="button"
              onClick={() => setPicking(true)}
              className="inline-flex h-10 items-center gap-1.5 rounded-full border-2 border-dashed border-ink-faint px-4 text-sm font-bold text-ink-soft hover:border-ink hover:text-ink"
            >
              <Plus className="size-4" />
              {t("form.addInvitees")}
            </button>
          </div>
          <FriendPicker
            open={picking}
            onOpenChange={setPicking}
            title={t("hangoutForm.pickInvitees")}
            initial={invitees.map((user) => user.id)}
            confirmLabel={t("picker.confirm")}
            onConfirm={(users) => {
              setInvitees(users);
              setPicking(false);
            }}
          />
        </div>
      )}
      <Button type="submit" size="lg" block loading={save.isPending} disabled={!name.trim()}>
        {t(hangout ? "form.save" : "hangoutForm.create")}
      </Button>
    </form>
  );
}
