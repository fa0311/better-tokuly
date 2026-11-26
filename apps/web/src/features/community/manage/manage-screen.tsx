import { Link } from "@tanstack/react-router";
import { community } from "@tokuly/api";
import { can, communityHref } from "@tokuly/core/community";
import {
  Ban,
  ChevronRight,
  Hash,
  Inbox,
  ScrollText,
  Settings,
  ShieldCheck,
  ShieldQuestion,
  UserPlus,
} from "lucide-react";
import type { ComponentType } from "react";
import { useTranslation } from "react-i18next";
import { CountBadge } from "#/components/ui/badge";
import { EmptyState, Page } from "#/components/ui/page";
import { NS } from "../api";
import { useCurrentCommunity } from "../context";
import { BansSection } from "./bans";
import { ChannelsSection } from "./channels";
import { InvitesSection } from "./invites";
import { LogSection } from "./log";
import { QueueSection } from "./queue";
import { RequestsSection } from "./requests";
import { SafetySection } from "./safety";
import { SettingsSection } from "./settings";

const P = community.Permission;

interface Section {
  id: string;
  permission: community.Permission;
  icon: ComponentType<{ className?: string }>;
  Component: ComponentType<{ community: community.Community }>;
  /** What waits for the staff there. */
  count?(viewer: community.CommunityViewer): number;
}

/** The staff pages, in the order of the menu. Each opens to whoever holds its permission. */
const SECTIONS: Section[] = [
  {
    id: "requests",
    permission: P.MANAGE_MEMBERS,
    icon: Inbox,
    Component: RequestsSection,
    count: (viewer) => viewer.pendingRequestCount,
  },
  {
    id: "queue",
    permission: P.MODERATE_CONTENT,
    icon: ShieldQuestion,
    Component: QueueSection,
    count: (viewer) => viewer.reviewQueueCount,
  },
  { id: "invites", permission: P.CREATE_INVITE, icon: UserPlus, Component: InvitesSection },
  { id: "channels", permission: P.MANAGE_CHANNELS, icon: Hash, Component: ChannelsSection },
  { id: "settings", permission: P.MANAGE_COMMUNITY, icon: Settings, Component: SettingsSection },
  { id: "safety", permission: P.MANAGE_SAFETY, icon: ShieldCheck, Component: SafetySection },
  { id: "bans", permission: P.MANAGE_MEMBERS, icon: Ban, Component: BansSection },
  { id: "log", permission: P.VIEW_MODERATION_LOG, icon: ScrollText, Component: LogSection },
];

/** `/g/<handle>/manage`: what the viewer may run here. */
export function ManageScreen() {
  const { t } = useTranslation(NS);
  const c = useCurrentCommunity();
  const sections = SECTIONS.filter((section) => can(c, section.permission));
  return (
    <Page title={t("manage.title")} back={communityHref(c.handle)}>
      {sections.length === 0 ? (
        <EmptyState emoji="🔒" title={t("manage.denied")} body={t("manage.deniedBody")} />
      ) : (
        <nav className="rounded-card bg-surface p-1.5">
          {sections.map(({ id, icon: Icon, count }) => (
            <Link
              key={id}
              to="/g/$handle/manage/$section"
              params={{ handle: c.handle, section: id }}
              className="flex items-center gap-3 rounded-2xl px-3 py-3 transition-colors hover:bg-surface-2"
            >
              <span className="grid size-10 shrink-0 place-items-center rounded-2xl bg-surface-2">
                <Icon className="size-5" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block font-bold">{t(`manage.${id}.title`)}</span>
                <span className="block text-sm text-ink-soft">{t(`manage.${id}.lead`)}</span>
              </span>
              {count && c.viewer && <CountBadge count={count(c.viewer)} />}
              <ChevronRight className="size-5 text-ink-faint" />
            </Link>
          ))}
        </nav>
      )}
    </Page>
  );
}

/** `/g/<handle>/manage/<section>`: one staff page, for whoever holds its permission. */
export function ManageSectionScreen({ section: id }: { section: string }) {
  const { t } = useTranslation(NS);
  const c = useCurrentCommunity();
  const section = SECTIONS.find((each) => each.id === id);
  const back = `${communityHref(c.handle)}/manage`;
  if (!section || !can(c, section.permission))
    return (
      <Page title={t("manage.title")} back={back}>
        <EmptyState emoji="🔒" title={t("manage.denied")} body={t("manage.deniedBody")} />
      </Page>
    );
  // Members who may only invite came from the front page, not from the staff menu.
  const fromFront = id === "invites" && !can(c, P.MANAGE_MEMBERS);
  return (
    <Page title={t(`manage.${id}.title`)} back={fromFront ? communityHref(c.handle) : back}>
      <section.Component community={c} />
    </Page>
  );
}
