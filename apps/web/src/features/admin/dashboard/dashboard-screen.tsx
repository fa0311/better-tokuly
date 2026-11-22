import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type admin, toDate } from "@tokuly/api";
import { CircleAlert, CircleCheck, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { compactDuration } from "#/lib/format";
import { ADMIN_SECTIONS } from "#/seams/admin";
import { systemClient } from "../api";
import { formatBytes, formatCompact, formatCount, formatDateTime } from "../format";
import { useStaffAccess } from "../roles";
import { ConsolePage, Section } from "../ui/console-page";
import { ErrorState } from "../ui/states";
import { ToneChip } from "../ui/tone";
import { type BarDatum, BarList, StatTile } from "./charts";

/** `/admin`: the operations dashboard for admins, a signpost for other staff. */
export function DashboardScreen() {
  return useStaffAccess().admin ? <Dashboard /> : <StaffHome />;
}

function Dashboard() {
  const { t } = useTranslation("admin");
  const stats = useQuery({
    queryKey: ["admin", "stats"],
    queryFn: () => systemClient().getServerStats({}),
    staleTime: 0,
  });
  const data = stats.data;

  return (
    <ConsolePage
      title={t("dashboard.title")}
      actions={
        <>
          {data?.generatedAt && (
            <span className="hidden text-xs text-ink-faint sm:inline">
              {t("dashboard.generatedAt", { time: formatDateTime(data.generatedAt) })}
            </span>
          )}
          <Button
            size="sm"
            variant="secondary"
            icon={<RefreshCw className="size-4" />}
            loading={stats.isFetching}
            onClick={() => stats.refetch()}
          >
            {t("common.refresh")}
          </Button>
        </>
      }
    >
      {stats.isPending ? (
        <DashboardSkeleton />
      ) : !data ? (
        <ErrorState error={stats.error} onRetry={() => stats.refetch()} />
      ) : (
        // A refresh keeps the previous numbers on screen, dimmed, until the new ones arrive.
        <div
          className={cn("flex flex-col gap-4 transition-opacity", stats.isFetching && "opacity-60")}
        >
          <UsersBlock users={data.users} />
          <div className="grid gap-4 lg:grid-cols-2">
            <ModerationBlock moderation={data.moderation} />
            <DependenciesBlock dependencies={data.dependencies} />
          </div>
          <div className="grid items-start gap-4 lg:grid-cols-2">
            <ContentBlock content={data.content} />
            <QueuesBlock queues={data.queues} />
          </div>
        </div>
      )}
    </ConsolePage>
  );
}

function DashboardSkeleton() {
  return (
    <div className="flex flex-col gap-4" aria-busy>
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        {[0, 1, 2, 3, 4, 5, 6, 7].map((i) => (
          <Skeleton key={i} className="h-24" />
        ))}
      </div>
      <div className="grid gap-4 lg:grid-cols-2">
        <Skeleton className="h-56" />
        <Skeleton className="h-56" />
      </div>
    </div>
  );
}

function UsersBlock({ users }: { users?: admin.UserStats }) {
  const { t } = useTranslation("admin");
  if (!users) return null;
  const total = Number(users.total);
  const activity: BarDatum[] = [
    { key: "today", label: t("dashboard.users.activeToday"), value: Number(users.activeToday) },
    { key: "7d", label: t("dashboard.users.active7d"), value: Number(users.active7d) },
    { key: "30d", label: t("dashboard.users.active30d"), value: Number(users.active30d) },
  ].map((row) => ({
    ...row,
    display: formatCount(row.value),
    detail: t("dashboard.users.shareOfAccounts", {
      share: total > 0 ? Math.round((row.value / total) * 100) : 0,
    }),
  }));

  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <StatTile
          hero
          className="col-span-2"
          label={t("dashboard.users.total")}
          value={formatCompact(users.total)}
          detail={t("dashboard.users.totalDetail", { total: formatCount(users.total) })}
        />
        <StatTile label={t("dashboard.users.newToday")} value={formatCount(users.newToday)} />
        <StatTile label={t("dashboard.users.new7d")} value={formatCount(users.new7d)} />
        <StatTile label={t("dashboard.users.sessions")} value={formatCount(users.activeSessions)} />
        <StatTile label={t("dashboard.users.verified")} value={formatCount(users.verified)} />
        <StatTile label={t("dashboard.users.suspended")} value={formatCount(users.suspended)} />
        <StatTile
          label={t("dashboard.users.deletionScheduled")}
          value={formatCount(users.deletionScheduled)}
        />
      </div>
      <Section
        title={t("dashboard.users.activity")}
        aside={<span className="text-xs text-ink-faint">{t("dashboard.users.activityHint")}</span>}
      >
        <div className="py-2">
          <BarList label={t("dashboard.users.activity")} data={activity} max={Math.max(total, 1)} />
        </div>
      </Section>
    </div>
  );
}

function ModerationBlock({ moderation }: { moderation?: admin.ModerationStats }) {
  const { t } = useTranslation("admin");
  const open = moderation?.openCases ?? 0n;
  const oldest = toDate(moderation?.oldestOpenCaseAt);
  return (
    <Section
      title={t("dashboard.moderation.title")}
      aside={
        <Link to="/admin/moderation" className="text-xs font-bold text-brand hover:underline">
          {t("dashboard.moderation.openQueue")}
        </Link>
      }
    >
      <div className="grid grid-cols-2 gap-3 p-3 sm:grid-cols-4 lg:grid-cols-2 2xl:grid-cols-4">
        <StatTile
          label={t("dashboard.moderation.openCases")}
          value={formatCount(open)}
          tone={open > 0n ? "alert" : undefined}
        />
        <StatTile
          label={t("dashboard.moderation.unassigned")}
          value={formatCount(moderation?.unassignedCases ?? 0n)}
        />
        <StatTile
          label={t("dashboard.moderation.reports24h")}
          value={formatCount(moderation?.reports24h ?? 0n)}
        />
        <StatTile
          label={t("dashboard.moderation.resolved7d")}
          value={formatCount(moderation?.resolved7d ?? 0n)}
        />
      </div>
      <p className="border-t-2 border-line px-4 py-2.5 text-sm text-ink-soft">
        {oldest
          ? t("dashboard.moderation.oldest", {
              age: compactDuration(Date.now() - oldest.getTime()),
            })
          : t("dashboard.moderation.queueEmpty")}
      </p>
    </Section>
  );
}

function DependenciesBlock({ dependencies }: { dependencies: admin.DependencyStatus[] }) {
  const { t } = useTranslation("admin");
  const down = dependencies.filter((dependency) => !dependency.reachable).length;
  return (
    <Section
      title={t("dashboard.dependencies.title")}
      aside={
        <ToneChip tone={down ? "danger" : "good"}>
          {down
            ? t("dashboard.dependencies.someDown", { count: down })
            : t("dashboard.dependencies.allUp")}
        </ToneChip>
      }
    >
      <ul className="divide-y divide-line">
        {dependencies.map((dependency) => (
          <li key={dependency.name} className="flex items-center gap-3 px-4 py-2 text-sm">
            {dependency.reachable ? (
              <CircleCheck className="size-5 shrink-0 text-success" aria-hidden />
            ) : (
              <CircleAlert className="size-5 shrink-0 text-danger" aria-hidden />
            )}
            <span className="min-w-0 flex-1">
              <span className="block font-bold">{dependency.name}</span>
              {!dependency.reachable && dependency.detail && (
                <span className="block truncate text-xs text-danger">{dependency.detail}</span>
              )}
            </span>
            <span className="text-xs font-bold text-ink-soft">
              {dependency.reachable
                ? t("dashboard.dependencies.up")
                : t("dashboard.dependencies.down")}
            </span>
            <span className="w-16 text-right tabular-nums text-ink-soft">
              {t("dashboard.dependencies.latency", { ms: formatCount(dependency.latencyMs) })}
            </span>
          </li>
        ))}
      </ul>
    </Section>
  );
}

function ContentBlock({ content }: { content: admin.ContentCount[] }) {
  const { t, i18n } = useTranslation("admin");
  const ranked: BarDatum[] = [...content]
    .sort((a, b) => Number(b.count - a.count))
    .map((entry) => ({
      key: entry.name,
      label: i18n.exists(`admin:dashboard.content.names.${entry.name}`)
        ? t(`dashboard.content.names.${entry.name}`)
        : entry.name,
      value: Number(entry.count),
      display: `${entry.approximate ? "≈" : ""}${formatCount(entry.count)}`,
      detail: entry.approximate ? t("dashboard.content.approximate") : entry.name,
    }));
  return (
    <Section
      title={t("dashboard.content.title")}
      aside={<span className="text-xs text-ink-faint">{t("dashboard.content.hint")}</span>}
    >
      <div className="py-2">
        <BarList label={t("dashboard.content.title")} data={ranked} />
      </div>
    </Section>
  );
}

function QueuesBlock({ queues }: { queues?: admin.QueueStats }) {
  const { t } = useTranslation("admin");
  if (!queues) return null;
  const outboxAge = Number(queues.outboxOldestAgeSeconds);
  return (
    <Section title={t("dashboard.queues.title")}>
      <div className="grid grid-cols-2 gap-3 p-3">
        <StatTile
          label={t("dashboard.queues.outboxPending")}
          value={formatCount(queues.outboxPending)}
          tone={queues.outboxPending > 100n ? "alert" : undefined}
        />
        <StatTile
          label={t("dashboard.queues.outboxOldest")}
          value={queues.outboxPending > 0n ? compactDuration(outboxAge * 1000) : "—"}
        />
      </div>
      <div className="flex flex-col">
        {queues.streams.map((stream) => (
          <StreamTable key={stream.name} stream={stream} />
        ))}
      </div>
    </Section>
  );
}

/** One event stream and how far each of its consumers is behind. */
function StreamTable({ stream }: { stream: admin.StreamStats }) {
  const { t } = useTranslation("admin");
  const lag = stream.consumers.reduce((sum, consumer) => sum + consumer.pending, 0n);
  return (
    <div className="border-t-2 border-line">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 px-4 pt-3 pb-1">
        <h3 className="font-black">{stream.name}</h3>
        <span className="text-xs text-ink-soft">
          {t("dashboard.queues.streamSize", {
            messages: formatCount(stream.messages),
            bytes: formatBytes(stream.bytes),
          })}
        </span>
        <span className="flex-1" />
        <ToneChip tone={lag > 0n ? "warn" : "neutral"}>
          {lag > 0n
            ? t("dashboard.queues.lagging", { lag: formatCount(lag) })
            : t("dashboard.queues.caughtUp")}
        </ToneChip>
      </div>
      {stream.consumers.length === 0 ? (
        <p className="px-4 pb-3 text-sm text-ink-faint">{t("dashboard.queues.noConsumers")}</p>
      ) : (
        <table
          className="mb-2 w-full text-sm"
          aria-label={t("dashboard.queues.consumersOf", { stream: stream.name })}
        >
          <thead>
            <tr className="text-xs text-ink-faint">
              <th scope="col" className="px-4 py-1 text-left font-bold">
                {t("dashboard.queues.consumer")}
              </th>
              <th scope="col" className="px-2 py-1 text-right font-bold">
                {t("dashboard.queues.pending")}
              </th>
              <th scope="col" className="px-2 py-1 text-right font-bold">
                {t("dashboard.queues.ackPending")}
              </th>
              <th scope="col" className="py-1 pr-4 pl-2 text-right font-bold">
                {t("dashboard.queues.redelivered")}
              </th>
            </tr>
          </thead>
          <tbody>
            {stream.consumers.map((consumer) => (
              <tr key={consumer.name} className="hover:bg-surface-2">
                <th
                  scope="row"
                  className="max-w-0 truncate px-4 py-1 text-left font-medium text-ink-soft"
                >
                  {consumer.name}
                </th>
                <td
                  className={cn(
                    "px-2 py-1 text-right tabular-nums",
                    consumer.pending > 0n && "font-black",
                  )}
                >
                  {formatCount(consumer.pending)}
                </td>
                <td className="px-2 py-1 text-right tabular-nums">
                  {formatCount(consumer.ackPending)}
                </td>
                <td
                  className={cn(
                    "py-1 pr-4 pl-2 text-right tabular-nums",
                    consumer.redelivered > 0n && "font-black text-danger",
                  )}
                >
                  {formatCount(consumer.redelivered)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

/** Staff without the admin role see no server numbers: point them at what they can open. */
function StaffHome() {
  const { t } = useTranslation("admin");
  const access = useStaffAccess();
  const sections = ADMIN_SECTIONS.filter((item) => item.to !== "/admin" && access[item.needs]);
  return (
    <ConsolePage title={t("title")}>
      <div className="mx-auto max-w-2xl">
        <p className="text-2xl font-black">{t("home.greeting")}</p>
        <p className="mt-1 text-sm text-ink-soft">
          {t(access.moderator ? "home.moderatorLead" : "home.staffLead")}
        </p>
        <ul className="mt-5 grid grid-cols-2 gap-3">
          {sections.map((item) => {
            const Icon = item.icon;
            return (
              <li key={item.to}>
                <Link
                  to={item.to}
                  className="pressable sticker flex h-24 flex-col justify-between rounded-card bg-surface p-4"
                >
                  <span className="grid size-9 place-items-center rounded-xl border-2 border-ink bg-coral text-[#17122b]">
                    <Icon className="size-5" />
                  </span>
                  <span className="font-black">{item.label()}</span>
                </Link>
              </li>
            );
          })}
        </ul>
      </div>
    </ConsolePage>
  );
}
