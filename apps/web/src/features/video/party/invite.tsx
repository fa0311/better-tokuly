import { useMutation, useQuery } from "@tanstack/react-query";
import { social } from "@tokuly/api";
import { friendKeys } from "@tokuly/core/friends";
import { Check, Copy, Send } from "lucide-react";
import QRCode from "qrcode";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { Skeleton } from "#/components/ui/page";
import { EmojiText } from "#/features/emoji";
import { api } from "#/lib/api";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { partyClient } from "../api";

/** Friends that can be invited in one go. */
const MAX_INVITES = 20;

/**
 * The link people join a party with. It opens a page that works with or
 * without an account: signed-in people land in the room, others can join as
 * a guest when the party allows it.
 */
export function partyLink(code: string): string {
  return `${window.location.origin}/join/party/${code}`;
}

/**
 * Bringing people into a party: the link (to copy, or as a QR code for the
 * person sitting next to you) and — for members with an account — friends to
 * notify.
 */
export function InviteDialog({
  open,
  onOpenChange,
  partyId,
  code,
  allowGuests,
  canInviteFriends,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  partyId: string;
  code: string;
  allowGuests: boolean /** Guests have no friends list. */;
  canInviteFriends: boolean;
}) {
  const { t } = useTranslation("video");
  const link = partyLink(code);
  const [qr, setQr] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [picked, setPicked] = useState<string[]>([]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void QRCode.toDataURL(link, {
      margin: 1,
      width: 360,
      color: { dark: "#17122b", light: "#ffffff" },
    }).then((dataUrl) => {
      if (!cancelled) setQr(dataUrl);
    });
    return () => {
      cancelled = true;
    };
  }, [open, link]);

  const friends = useQuery({
    queryKey: friendKeys.list(""),
    queryFn: () =>
      api()
        .service(social.FriendService)
        .listFriends({ query: "", page: { limit: 100 } }),
    enabled: open && canInviteFriends,
  });
  const invite = useMutation({
    mutationFn: () => partyClient().inviteToParty({ partyId, userIds: picked }),
    onSuccess: () => {
      toast.success(t("party.invite.sent", { count: picked.length }));
      setPicked([]);
    },
    onError: (error) => toast.error(errorMessage(error)),
  });

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(link);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
      toast.success(t("actions.copied"));
    } catch {
      toast.error(t("party.invite.copyFailed"));
    }
  };
  const toggle = (userId: string) =>
    setPicked((current) =>
      current.includes(userId)
        ? current.filter((id) => id !== userId)
        : current.length < MAX_INVITES
          ? [...current, userId]
          : current,
    );
  const people =
    friends.data?.friends.flatMap((friend) => (friend.user ? [friend.user] : [])) ?? [];

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("party.invite.title")}
      description={allowGuests ? t("party.invite.leadGuests") : t("party.invite.leadMembers")}
    >
      <div className="flex flex-col gap-5">
        <div className="flex flex-col items-center gap-3 sm:flex-row sm:items-start">
          <div className="sticker grid size-36 shrink-0 place-items-center rounded-3xl bg-white p-2">
            {qr ? (
              <img src={qr} alt={t("party.invite.qr")} className="size-full" />
            ) : (
              <Skeleton className="size-full" />
            )}
          </div>
          <div className="flex min-w-0 flex-1 flex-col gap-2 self-stretch">
            <p
              className="rounded-2xl bg-surface-2 px-3 py-2 text-sm font-bold break-all"
              data-testid="party-link"
            >
              {link}
            </p>
            <Button
              variant="secondary"
              icon={copied ? <Check className="size-4" /> : <Copy className="size-4" />}
              onClick={() => void copy()}
            >
              {t("party.invite.copy")}
            </Button>
          </div>
        </div>

        {canInviteFriends && (
          <section>
            <h3 className="font-black">{t("party.invite.friends")}</h3>
            {friends.isPending ? (
              <Skeleton className="mt-2 h-14" />
            ) : people.length === 0 ? (
              <p className="mt-1 text-sm text-ink-faint">{t("party.invite.noFriends")}</p>
            ) : (
              <>
                <ul className="mt-2 flex max-h-56 flex-col overflow-y-auto">
                  {people.map((user) => {
                    const selected = picked.includes(user.id);
                    return (
                      <li key={user.id}>
                        {/* biome-ignore lint/a11y/useSemanticElements: a row-sized tick; a native checkbox cannot carry this layout. */}
                        <button
                          type="button"
                          role="checkbox"
                          aria-checked={selected}
                          className="flex w-full items-center gap-3 rounded-2xl px-2 py-2 text-left hover:bg-surface-2"
                          onClick={() => toggle(user.id)}
                        >
                          <Avatar user={user} size="sm" />
                          <span className="min-w-0 flex-1">
                            <span className="block truncate font-bold">
                              <EmojiText text={user.displayName} />
                            </span>
                            <span className="block truncate text-xs text-ink-soft">
                              @{user.handle}
                            </span>
                          </span>
                          <span
                            className={cn(
                              "grid size-6 shrink-0 place-items-center rounded-lg border-2 border-ink",
                              selected ? "bg-lime text-[#17122b]" : "bg-surface",
                            )}
                          >
                            {selected && <Check className="size-4" strokeWidth={3} />}
                          </span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
                <Button
                  className="mt-3"
                  block
                  icon={<Send className="size-4" />}
                  disabled={picked.length === 0}
                  loading={invite.isPending}
                  onClick={() => invite.mutate()}
                >
                  {picked.length > 0
                    ? t("party.invite.send", { count: picked.length })
                    : t("party.invite.pick")}
                </Button>
              </>
            )}
          </section>
        )}
      </div>
    </Dialog>
  );
}
