import { useMutation } from "@tanstack/react-query";
import { video } from "@tokuly/api";
import { Crown, MoreVertical, ShieldMinus, ShieldPlus, UserX } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { partyClient } from "../api";
import { MemberAvatar } from "./chat";
import type { PartyRoom } from "./use-party";

const { PartyRole } = video;

/**
 * Who is in the party, host first. The host can make members co-hosts, hand
 * the party over and remove people; co-hosts can remove plain members.
 */
export function PartyMembers({ room }: { room: PartyRoom }) {
  const { t } = useTranslation("video");
  const { members, self, party } = room.state;
  const [confirming, setConfirming] = useState<{
    kind: "remove" | "handOver";
    member: video.PartyMember;
  } | null>(null);
  const isHost = self.role === PartyRole.HOST;
  const isCohost = self.role === PartyRole.COHOST;
  const onError = (error: unknown) => {
    setConfirming(null);
    toast.error(errorMessage(error, { namespace: "video" }));
  };

  const setRole = useMutation({
    mutationFn: ({ member, role }: { member: video.PartyMember; role: video.PartyRole }) =>
      partyClient().setPartyMemberRole({ partyId: party.id, memberId: member.id, role }),
    onSuccess: () => {
      setConfirming(null);
      void room.reload();
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: (member: video.PartyMember) =>
      partyClient().removePartyMember({ partyId: party.id, memberId: member.id }),
    onSuccess: (_response, member) => {
      setConfirming(null);
      toast.success(t("party.members.removed", { name: member.name }));
      void room.reload();
    },
    onError,
  });

  const actionsFor = (member: video.PartyMember): MenuAction[] => {
    if (member.id === self.id) return [];
    const actions: MenuAction[] = [];
    if (isHost && !member.isGuest) {
      actions.push(
        member.role === PartyRole.COHOST
          ? {
              label: t("party.members.removeCohost"),
              icon: <ShieldMinus />,
              onSelect: () => setRole.mutate({ member, role: PartyRole.MEMBER }),
            }
          : {
              label: t("party.members.makeCohost"),
              icon: <ShieldPlus />,
              onSelect: () => setRole.mutate({ member, role: PartyRole.COHOST }),
            },
        {
          label: t("party.members.handOver"),
          icon: <Crown />,
          onSelect: () => setConfirming({ kind: "handOver", member }),
        },
      );
    }
    if (isHost || (isCohost && member.role === PartyRole.MEMBER))
      actions.push({
        label: t("party.members.remove"),
        icon: <UserX />,
        danger: true,
        onSelect: () => setConfirming({ kind: "remove", member }),
      });
    return actions;
  };

  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
      <p className="px-2 pb-1 text-xs font-bold text-ink-faint">
        {t("party.members.count", { count: members.length, max: party.maxMembers })}
      </p>
      <ul>
        {members.map((member) => {
          const actions = actionsFor(member);
          return (
            <li
              key={member.id}
              data-testid="party-member"
              className="flex items-center gap-3 rounded-2xl px-2 py-2"
            >
              <MemberAvatar member={member} size="md" />
              <div className="min-w-0 flex-1">
                <p className="flex items-center gap-1.5">
                  <span className="truncate font-bold">{member.name}</span>
                  {member.id === self.id && (
                    <span className="shrink-0 text-xs font-bold text-ink-faint">
                      {t("party.members.you")}
                    </span>
                  )}
                </p>
                <p className="flex flex-wrap gap-1.5 text-xs font-black">
                  {member.role === PartyRole.HOST && (
                    <span className="inline-flex h-5 items-center gap-1 rounded-full bg-sun px-2 text-[#17122b]">
                      <Crown className="size-3" />
                      {t("party.role.host")}
                    </span>
                  )}
                  {member.role === PartyRole.COHOST && (
                    <span className="inline-flex h-5 items-center rounded-full bg-sky px-2 text-[#17122b]">
                      {t("party.role.cohost")}
                    </span>
                  )}
                  {member.isGuest && (
                    <span className="inline-flex h-5 items-center rounded-full bg-surface-2 px-2 text-ink-soft">
                      {t("party.role.guest")}
                    </span>
                  )}
                </p>
              </div>
              {actions.length > 0 && (
                <Menu
                  trigger={
                    <button
                      type="button"
                      aria-label={t("party.members.menu", { name: member.name })}
                      className="grid size-9 shrink-0 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
                    >
                      <MoreVertical className="size-4" />
                    </button>
                  }
                  actions={actions}
                />
              )}
            </li>
          );
        })}
      </ul>

      <ConfirmDialog
        open={confirming?.kind === "remove"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t("party.members.confirmRemove", { name: confirming?.member.name ?? "" })}
        description={t("party.members.confirmRemoveBody")}
        confirmLabel={t("party.members.remove")}
        danger
        loading={remove.isPending}
        onConfirm={() => confirming && remove.mutate(confirming.member)}
      />
      <ConfirmDialog
        open={confirming?.kind === "handOver"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t("party.members.confirmHandOver", { name: confirming?.member.name ?? "" })}
        description={t("party.members.confirmHandOverBody")}
        confirmLabel={t("party.members.handOver")}
        loading={setRole.isPending}
        onConfirm={() =>
          confirming && setRole.mutate({ member: confirming.member, role: PartyRole.HOST })
        }
      />
    </div>
  );
}
