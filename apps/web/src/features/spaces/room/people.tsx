import { Link } from "@tanstack/react-router";
import { type common, spaces } from "@tokuly/api";
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  Ban,
  Crown,
  Hand,
  MicOff,
  ShieldCheck,
  ShieldMinus,
  UserRound,
  UserX,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { Avatar } from "#/components/ui/avatar";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { cn } from "#/lib/cn";
import { useMe } from "#/stores/session";
import { spaceClient } from "../api";
import { isModerator, nameOf, onStage } from "../model";
import { PersonName } from "../parts";
import type { Voices } from "../session";
import type { RoomAction } from "./actions";

const Role = spaces.SpaceRole;

function roleKey(role: spaces.SpaceRole): "host" | "cohost" | "speaker" | "listener" {
  if (role === Role.HOST) return "host";
  if (role === Role.COHOST) return "cohost";
  if (role === Role.SPEAKER) return "speaker";
  return "listener";
}

/** Someone on stage: a big face that lights up while they talk. */
export function StageTile({
  participant,
  voices,
  onOpen,
}: {
  participant: spaces.SpaceParticipant;
  voices: Voices;
  onOpen(): void;
}) {
  const { t } = useTranslation("spaces");
  const user = participant.user;
  const id = user?.id ?? "";
  const speaking = voices.speaking.has(id);
  // Outside the room there is no connection to tell who is muted.
  const muted = voices.connected.has(id) && !voices.open.has(id);
  const role = roleKey(participant.role);
  return (
    <li>
      <button
        type="button"
        onClick={onOpen}
        className="group flex w-full flex-col items-center gap-1.5 rounded-2xl p-1.5 text-center hover:bg-surface-2"
        data-testid="stage-person"
        data-speaking={speaking || undefined}
        data-mic={voices.open.has(id) ? "on" : undefined}
        aria-label={t(`room.personLabel.${role}`, { name: nameOf(user) })}
      >
        <span className="relative">
          {speaking && (
            <span className="absolute -inset-1 animate-pulse-ring rounded-[1.4rem] bg-lime" />
          )}
          <Avatar
            user={user}
            size="lg"
            outlined
            className={cn(
              "relative rounded-[1.25rem] transition-shadow",
              speaking && "ring-4 ring-lime",
            )}
          />
          {muted && (
            <span
              className="absolute -right-1.5 -bottom-1.5 grid size-6 place-items-center rounded-full border-2 border-ink bg-surface text-ink-soft"
              role="img"
              aria-label={t("room.mutedLabel")}
            >
              <MicOff className="size-3.5" />
            </span>
          )}
        </span>
        <span className="w-full truncate text-sm font-bold">
          <PersonName user={user} />
        </span>
        <span
          className={cn(
            "rounded-full px-2 text-[11px] font-black",
            participant.role === Role.HOST
              ? "bg-sun text-[#17122b]"
              : participant.role === Role.COHOST
                ? "bg-sky text-[#17122b]"
                : "bg-surface-2 text-ink-soft",
          )}
        >
          {t(`role.${role}`)}
        </span>
      </button>
    </li>
  );
}

/** Someone in the audience: a small face, a raised hand when it is up. */
export function AudienceTile({
  participant,
  onOpen,
}: {
  participant: spaces.SpaceParticipant;
  onOpen(): void;
}) {
  const { t } = useTranslation("spaces");
  const raised = Boolean(participant.handRaisedAt);
  return (
    <li>
      <button
        type="button"
        onClick={onOpen}
        className="flex w-full flex-col items-center gap-1 rounded-2xl p-1.5 hover:bg-surface-2"
        data-testid="audience-person"
        aria-label={t("room.personLabel.listener", { name: nameOf(participant.user) })}
      >
        <span className="relative">
          <Avatar user={participant.user} outlined />
          {raised && (
            <span
              className="absolute -top-2 -right-2 animate-wiggle text-lg"
              role="img"
              aria-label={t("room.handUpLabel")}
            >
              ✋
            </span>
          )}
        </span>
        <span className="w-full truncate text-xs font-bold text-ink-soft">
          <PersonName user={participant.user} />
        </span>
      </button>
    </li>
  );
}

/**
 * What the viewer can do about one person in the room: see their profile
 * and, as a moderator, bring them on stage, send them back, mute, name a
 * co-host, hand the room over, remove or ban.
 */
export function PersonSheet({
  space,
  participant,
  onClose,
  run,
}: {
  space: spaces.Space;
  participant: spaces.SpaceParticipant | null;
  onClose(): void;
  run(action: RoomAction): void;
}) {
  const { t } = useTranslation("spaces");
  const myId = useMe()?.user?.id ?? "";
  const [confirming, setConfirming] = useState<"remove" | "ban" | "transfer" | null>(null);
  // The sheet keeps showing the person while it slides away.
  const [last, setLast] = useState(participant);
  if (participant && participant !== last) setLast(participant);
  const shown = participant ?? last;
  const user = shown?.user;
  if (!shown || !user) return null;

  const spaceId = space.id;
  const userId = user.id;
  const name = nameOf(user);
  const myRole = space.viewer?.role ?? Role.UNSPECIFIED;
  const live = space.state === spaces.SpaceState.LIVE;
  const self = userId === myId;
  const iAmHost = myRole === Role.HOST;
  // The host cannot be touched; a co-host only by the host.
  const mayModerate =
    live &&
    !self &&
    isModerator(myRole) &&
    shown.role !== Role.HOST &&
    (shown.role !== Role.COHOST || iAmHost);
  const staged = onStage(shown.role);
  const act = (action: RoomAction) => {
    run(action);
    setConfirming(null);
    onClose();
  };

  return (
    <>
      <Dialog
        open={participant !== null && confirming === null}
        onOpenChange={(open) => !open && onClose()}
        title={
          <span className="flex items-center gap-3">
            <Avatar user={user} outlined />
            <span className="min-w-0">
              <span className="block truncate">
                <PersonName user={user} />
              </span>
              <span className="block truncate text-sm font-bold text-ink-soft">
                @{user.handle}・{t(`role.${roleKey(shown.role)}`)}
              </span>
            </span>
          </span>
        }
      >
        <ul className="flex flex-col gap-1">
          <li>
            <Link
              to="/u/$handle"
              params={{ handle: user.handle }}
              className={ROW}
              onClick={onClose}
            >
              <UserRound className="size-5" />
              {t("person.profile")}
            </Link>
          </li>
          {mayModerate && !staged && (
            <>
              <Row
                icon={<ArrowUpFromLine />}
                label={t(shown.handRaisedAt ? "person.bringUp" : "person.inviteToStage")}
                onSelect={() =>
                  act({
                    call: async () => {
                      const response = await spaceClient().inviteToStage({ spaceId, userId });
                      return response.onStage;
                    },
                    done: t(shown.handRaisedAt ? "toast.onStage" : "toast.stageInvited", { name }),
                  })
                }
              />
              {(shown.stageInvited || shown.handRaisedAt) && (
                <Row
                  icon={<Hand />}
                  label={t(shown.stageInvited ? "person.withdrawInvite" : "person.lowerHand")}
                  onSelect={() =>
                    act({ call: () => spaceClient().moveToAudience({ spaceId, userId }) })
                  }
                />
              )}
            </>
          )}
          {mayModerate && staged && (
            <>
              <Row
                icon={<MicOff />}
                label={t("person.mute")}
                onSelect={() =>
                  act({
                    call: () => spaceClient().muteSpeaker({ spaceId, userId }),
                    done: t("toast.muted", { name }),
                  })
                }
              />
              <Row
                icon={<ArrowDownToLine />}
                label={t("person.toAudience")}
                onSelect={() =>
                  act({ call: () => spaceClient().moveToAudience({ spaceId, userId }) })
                }
              />
            </>
          )}
          {live && iAmHost && !self && (
            <>
              <Row
                icon={shown.role === Role.COHOST ? <ShieldMinus /> : <ShieldCheck />}
                label={t(shown.role === Role.COHOST ? "person.removeCohost" : "person.makeCohost")}
                onSelect={() =>
                  act({
                    call: () =>
                      spaceClient().setCohost({
                        spaceId,
                        userId,
                        cohost: shown.role !== Role.COHOST,
                      }),
                    done: shown.role === Role.COHOST ? undefined : t("toast.cohostNamed", { name }),
                  })
                }
              />
              <Row
                icon={<Crown />}
                label={t("person.transferHost")}
                onSelect={() => setConfirming("transfer")}
              />
            </>
          )}
          {mayModerate && (
            <>
              <Row
                icon={<UserX />}
                label={t("person.remove")}
                danger
                onSelect={() => setConfirming("remove")}
              />
              <Row
                icon={<Ban />}
                label={t("person.ban")}
                danger
                onSelect={() => setConfirming("ban")}
              />
            </>
          )}
        </ul>
      </Dialog>
      <ConfirmDialog
        open={confirming === "remove" || confirming === "ban"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t(confirming === "ban" ? "confirm.banTitle" : "confirm.removeTitle", { name })}
        description={t(confirming === "ban" ? "confirm.banBody" : "confirm.removeBody")}
        confirmLabel={t(confirming === "ban" ? "person.ban" : "person.remove")}
        danger
        onConfirm={() =>
          act({
            call: () =>
              spaceClient().removeParticipant({ spaceId, userId, ban: confirming === "ban" }),
            done: t(confirming === "ban" ? "toast.banned" : "toast.removed", { name }),
          })
        }
      />
      <ConfirmDialog
        open={confirming === "transfer"}
        onOpenChange={(open) => !open && setConfirming(null)}
        title={t("confirm.transferTitle", { name })}
        description={t("confirm.transferBody")}
        confirmLabel={t("person.transferHost")}
        onConfirm={() =>
          act({
            call: async () => {
              const response = await spaceClient().transferHost({ spaceId, userId });
              return response.space;
            },
            done: t("toast.transferred", { name }),
          })
        }
      />
    </>
  );
}

const ROW =
  "flex w-full items-center gap-3 rounded-2xl px-3 py-3 text-left font-bold transition-colors hover:bg-surface-2 [&>svg]:size-5";

function Row({
  icon,
  label,
  danger,
  onSelect,
}: {
  icon: ReactNode;
  label: string;
  danger?: boolean;
  onSelect(): void;
}) {
  return (
    <li>
      <button type="button" onClick={onSelect} className={cn(ROW, danger && "text-danger")}>
        {icon}
        {label}
      </button>
    </li>
  );
}

/** A plain row of a person in a list (invited, banned, gone), with a control at the end. */
export function PersonRow({
  user,
  note,
  trailing,
}: {
  user: common.UserSummary | undefined;
  note?: ReactNode;
  trailing?: ReactNode;
}) {
  return (
    <li className="flex items-center gap-3 rounded-2xl px-2 py-2">
      <Avatar user={user} outlined />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-bold">
          <PersonName user={user} />
        </span>
        <span className="block truncate text-sm text-ink-soft">{note ?? `@${user?.handle}`}</span>
      </span>
      {trailing}
    </li>
  );
}
