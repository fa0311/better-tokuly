import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { common, community } from "@tokuly/api";
import {
  can,
  communityHref,
  communityKeys,
  imageUrl,
  inviteCodeOf,
  isStaff,
  refreshMembership,
  storeCommunity,
} from "@tokuly/core/community";
import {
  Bell,
  CheckCheck,
  Clock,
  DoorOpen,
  MoreHorizontal,
  Settings,
  ShieldAlert,
  UserPlus,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Chip } from "#/components/ui/card";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Dialog } from "#/components/ui/dialog";
import { TextArea, TextField } from "#/components/ui/field";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { SelectField } from "#/components/ui/select";
import { SwitchRow } from "#/components/ui/switch";
import { EmojiText } from "#/features/emoji";
import { errorMessage } from "#/lib/errors";
import { boardClient, communityClient, NS } from "./api";
import {
  CommunityIcon,
  categoryOf,
  MemberCount,
  Notice,
  RoleChip,
  TagList,
  useWhen,
  VisibilityChip,
} from "./parts";
import { useShareMenu } from "./share";

const S = community.MembershipStatus;
const V = community.CommunityVisibility;
const P = community.Permission;

/** The top of a community's front page: what it is, and the viewer's way in (or their own settings). */
export function CommunityHeader({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const banner = imageUrl(c.banner, 640);
  const category = categoryOf(c.category);
  return (
    <section className="sticker overflow-hidden rounded-card bg-surface">
      <div
        className="h-28 bg-brand-soft sm:h-40"
        style={
          banner
            ? {
                backgroundImage: `url("${banner}")`,
                backgroundSize: "cover",
                backgroundPosition: "center",
              }
            : undefined
        }
      />
      <div className="px-4 pb-4 sm:px-5">
        <div className="-mt-10 flex items-end gap-3">
          <CommunityIcon community={c} size="lg" />
          <div className="ml-auto flex items-center gap-2 pb-1">
            <MembershipControls community={c} />
          </div>
        </div>
        <h2 className="mt-3 text-2xl font-black leading-tight">
          <EmojiText text={c.name} />
        </h2>
        <p className="text-sm font-bold text-ink-faint">+{c.handle}</p>
        <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1.5">
          <Chip>
            <span aria-hidden>{category.emoji}</span>
            {t(`category.${category.key}`)}
          </Chip>
          <VisibilityChip visibility={c.visibility} />
          {c.viewer && <RoleChip role={c.viewer.role} />}
          <MemberCount count={c.memberCount} />
          {c.schoolName && <Chip>🏫 {c.schoolName}</Chip>}
        </div>
        {c.description && (
          <p className="mt-3 whitespace-pre-wrap break-words text-[15px] leading-relaxed">
            <EmojiText text={c.description} />
          </p>
        )}
        <TagList tags={c.tags} className="mt-3" />
        <StandingNotice community={c} />
      </div>
    </section>
  );
}

/** What the viewer should know about their own standing: banned, timed out, waiting. */
function StandingNotice({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const when = useWhen();
  const viewer = c.viewer;
  if (!viewer) return null;
  if (viewer.status === S.BANNED) {
    const until = viewer.banExpiresAt ? Number(viewer.banExpiresAt.seconds) * 1000 : 0;
    return (
      <Notice tone="warn" icon={<ShieldAlert />} className="mt-4">
        <p className="font-bold">{t("ban.you")}</p>
        {viewer.banReason && <p>{t("ban.reason", { reason: viewer.banReason })}</p>}
        <p>{until ? t("ban.until", { time: when(until) }) : t("ban.forever")}</p>
      </Notice>
    );
  }
  const timeout = viewer.timeoutUntil ? Number(viewer.timeoutUntil.seconds) * 1000 : 0;
  if (timeout > Date.now())
    return (
      <Notice tone="warn" icon={<Clock />} className="mt-4">
        {t("timeout.you", { time: when(timeout) })}
      </Notice>
    );
  if (viewer.status === S.REQUESTED)
    return (
      <Notice icon={<Clock />} className="mt-4">
        {t("join.requestedHint")}
      </Notice>
    );
  return null;
}

/** Join / request / cancel for a visitor; the member's own menu for a member. */
export function MembershipControls({ community: c }: { community: community.Community }) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [asking, setAsking] = useState(false);
  const [settings, setSettings] = useState(false);
  const [leaving, setLeaving] = useState(false);
  const status = c.viewer?.status ?? S.NONE;
  const share = useShareMenu({
    path: communityHref(c.handle),
    title: c.name,
    subject: { type: common.SubjectType.COMMUNITY, id: c.id },
    share: c.visibility !== V.HIDDEN,
    report: c.viewer?.role !== community.MemberRole.OWNER,
  });

  const done = (joined: community.JoinCommunityResponse) => {
    if (joined.community) storeCommunity(queryClient, joined.community);
    refreshMembership(queryClient, c.id);
    toast.success(joined.status === S.MEMBER ? t("join.welcome") : t("join.requested"));
  };
  const join = useMutation({
    mutationFn: () => communityClient().joinCommunity({ communityId: c.id }),
    onSuccess: done,
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  const cancel = useMutation({
    mutationFn: () => communityClient().cancelJoinRequest({ communityId: c.id }),
    onSuccess: () => {
      refreshMembership(queryClient, c.id);
      toast(t("join.cancelled"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  const leave = useMutation({
    mutationFn: () => communityClient().leaveCommunity({ communityId: c.id }),
    onSuccess: () => {
      setLeaving(false);
      refreshMembership(queryClient, c.id);
      toast(t("leave.done"));
      if (c.visibility === V.HIDDEN) void navigate({ to: "/communities" });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  const markRead = useMutation({
    mutationFn: () => boardClient().markRead({ target: { case: "communityId", value: c.id } }),
    onSuccess: () => {
      refreshMembership(queryClient, c.id);
      toast.success(t("read.done"));
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });

  const more = (actions: MenuAction[]) =>
    actions.length > 0 && (
      <Menu
        trigger={
          <Button variant="secondary" size="icon-sm" aria-label={t("more")}>
            <MoreHorizontal className="size-5" />
          </Button>
        }
        actions={actions}
      />
    );

  if (status === S.MEMBER) {
    const own: MenuAction[] = [
      { label: t("notify.title"), icon: <Bell />, onSelect: () => setSettings(true) },
      { label: t("read.all"), icon: <CheckCheck />, onSelect: () => markRead.mutate() },
    ];
    if (c.viewer?.role !== community.MemberRole.OWNER)
      own.push({
        label: t("leave.action"),
        icon: <DoorOpen />,
        danger: true,
        onSelect: () => setLeaving(true),
      });
    return (
      <>
        {can(c, P.CREATE_INVITE) && (
          <Button
            variant="secondary"
            size="sm"
            icon={<UserPlus className="size-4" />}
            onClick={() =>
              void navigate({
                to: "/g/$handle/manage/$section",
                params: { handle: c.handle, section: "invites" },
              })
            }
          >
            {t("invite.action")}
          </Button>
        )}
        {isStaff(c) && (
          <span className="relative">
            <Button
              variant="secondary"
              size="icon-sm"
              aria-label={t("manage.title")}
              onClick={() =>
                void navigate({ to: "/g/$handle/manage", params: { handle: c.handle } })
              }
            >
              <Settings className="size-5" />
            </Button>
            {(c.viewer?.pendingRequestCount ?? 0) + (c.viewer?.reviewQueueCount ?? 0) > 0 && (
              <span className="pointer-events-none absolute -top-1 -right-1 size-3.5 rounded-full border-2 border-surface bg-pink" />
            )}
          </span>
        )}
        {more([...share.actions.slice(0, 2), ...own, ...share.actions.slice(2)])}
        {share.sheets}
        <MembershipSettings community={c} open={settings} onOpenChange={setSettings} />
        <ConfirmDialog
          open={leaving}
          onOpenChange={setLeaving}
          title={t("leave.title", { name: c.name })}
          description={c.visibility === V.PUBLIC ? t("leave.body") : t("leave.bodyClosed")}
          confirmLabel={t("leave.action")}
          danger
          loading={leave.isPending}
          onConfirm={() => leave.mutate()}
        />
      </>
    );
  }
  if (status === S.BANNED)
    return (
      <>
        {more(share.actions)}
        {share.sheets}
      </>
    );
  if (status === S.REQUESTED)
    return (
      <>
        <Button
          variant="secondary"
          size="sm"
          loading={cancel.isPending}
          onClick={() => cancel.mutate()}
        >
          {t("join.cancel")}
        </Button>
        {more(share.actions)}
        {share.sheets}
      </>
    );
  return (
    <>
      {c.visibility === V.PUBLIC ? (
        <Button size="sm" loading={join.isPending} onClick={() => join.mutate()}>
          {t("join.action")}
        </Button>
      ) : (
        <Button size="sm" onClick={() => setAsking(true)}>
          {t("join.request")}
        </Button>
      )}
      {more(share.actions)}
      {share.sheets}
      <JoinRequestDialog community={c} open={asking} onOpenChange={setAsking} onJoined={done} />
    </>
  );
}

/** Asking to join a community that is not open: its questions, or an invite code that lets one straight in. */
function JoinRequestDialog({
  community: c,
  open,
  onOpenChange,
  onJoined,
}: {
  community: community.Community;
  open: boolean;
  onOpenChange(open: boolean): void;
  onJoined(response: community.JoinCommunityResponse): void;
}) {
  const { t } = useTranslation(NS);
  const [answers, setAnswers] = useState<string[]>([]);
  const [code, setCode] = useState("");
  const inviteCode = inviteCodeOf(code);
  const join = useMutation({
    mutationFn: () =>
      communityClient().joinCommunity({
        communityId: c.id,
        inviteCode,
        answers: inviteCode ? [] : c.joinQuestions.map((_, index) => answers[index]?.trim() ?? ""),
      }),
    onSuccess: (response) => {
      onOpenChange(false);
      onJoined(response);
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  const complete =
    Boolean(inviteCode) || c.joinQuestions.every((_, index) => answers[index]?.trim());
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("join.requestTitle", { name: c.name })}
      description={t("join.requestBody")}
      footer={
        <Button loading={join.isPending} disabled={!complete} onClick={() => join.mutate()}>
          {inviteCode ? t("join.withCode") : t("join.send")}
        </Button>
      }
    >
      <div className="flex flex-col gap-4">
        {c.joinQuestions.map((question, index) => (
          <TextArea
            key={question}
            label={question}
            maxLength={500}
            value={answers[index] ?? ""}
            disabled={Boolean(inviteCode)}
            onChange={(event) =>
              setAnswers((current) => {
                const next = [...current];
                next[index] = event.target.value;
                return next;
              })
            }
          />
        ))}
        <TextField
          label={t("join.codeLabel")}
          hint={t("join.codeHint")}
          value={code}
          autoCapitalize="off"
          autoComplete="off"
          onChange={(event) => setCode(event.target.value)}
        />
      </div>
    </Dialog>
  );
}

const LEVELS = [
  { value: community.NotificationLevel.ALL, key: "all" },
  { value: community.NotificationLevel.MENTIONS, key: "mentions" },
  { value: community.NotificationLevel.NONE, key: "none" },
] as const;

/** The member's own settings here: what they are notified about, whether the membership shows on their profile. */
function MembershipSettings({
  community: c,
  open,
  onOpenChange,
}: {
  community: community.Community;
  open: boolean;
  onOpenChange(open: boolean): void;
}) {
  const { t } = useTranslation(NS);
  const queryClient = useQueryClient();
  const update = useMutation({
    mutationFn: (change: {
      notificationLevel?: community.NotificationLevel;
      showOnProfile?: boolean;
    }) => communityClient().updateMembership({ communityId: c.id, ...change }),
    onSuccess: ({ viewer }) => {
      if (viewer) storeCommunity(queryClient, { ...c, viewer });
      void queryClient.invalidateQueries({ queryKey: communityKeys.my });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: NS })),
  });
  const level = c.viewer?.notificationLevel || community.NotificationLevel.ALL;
  return (
    <Dialog open={open} onOpenChange={onOpenChange} title={t("notify.title")}>
      <div className="flex flex-col gap-3">
        <SelectField
          label={t("notify.level")}
          value={String(level)}
          onChange={(value) => update.mutate({ notificationLevel: Number(value) })}
          options={LEVELS.map(({ value, key }) => ({
            value: String(value),
            label: t(`notify.${key}`),
            hint: t(`notify.${key}Hint`),
          }))}
        />
        {c.visibility === V.PUBLIC && (
          <SwitchRow
            label={t("notify.showOnProfile")}
            hint={t("notify.showOnProfileHint")}
            checked={c.viewer?.showOnProfile ?? false}
            onChange={(showOnProfile) => update.mutate({ showOnProfile })}
          />
        )}
      </div>
    </Dialog>
  );
}
