import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { type identity, toDate } from "@tokuly/api";
import { ChevronRight } from "lucide-react";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { accountClient } from "#/lib/api";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import { keys } from "../data";
import { Group, SettingsPage } from "../shell/settings-page";

/** An emoji per kind of event; the label is `security.log.types.<TYPE>`. */
const EVENT_EMOJI: Record<string, string> = {
  SIGN_UP: "🎉",
  SIGN_IN: "🚪",
  SIGN_IN_FAILED: "⚠️",
  SIGN_OUT: "👋",
  PASSWORD_CHANGED: "🔑",
  PASSWORD_RESET: "🔑",
  EMAIL_CHANGED: "✉️",
  PASSKEY_ADDED: "🗝️",
  PASSKEY_REMOVED: "🗝️",
  TOTP_ENABLED: "🛡️",
  TOTP_DISABLED: "🛡️",
  RECOVERY_CODE_USED: "🧯",
  RECOVERY_CODES_REVOKED: "🧯",
  SESSION_REVOKED: "📴",
  DEVICE_LOGIN_APPROVED: "📲",
  DEVICE_LOGIN_DENIED: "🚫",
  DELETION_REQUESTED: "🗑️",
  DELETION_CANCELLED: "↩️",
  REFRESH_TOKEN_REUSE: "🚨",
};

/** Events worth a second look. */
const SUSPICIOUS = new Set(["SIGN_IN_FAILED", "REFRESH_TOKEN_REUSE"]);

function EventRow({ event }: { event: identity.SecurityEvent }) {
  const { t, i18n } = useTranslation("settings");
  const at = toDate(event.createdAt);
  const where = [event.deviceName, event.location || event.ip].filter(Boolean).join(" ・ ");
  return (
    <li data-testid="security-event" className="flex items-start gap-3 rounded-2xl px-3 py-2.5">
      <span
        aria-hidden
        className={cn(
          "grid size-10 shrink-0 place-items-center rounded-xl text-xl",
          SUSPICIOUS.has(event.type) ? "bg-sun" : "bg-surface-2",
        )}
      >
        {EVENT_EMOJI[event.type] ?? "📝"}
      </span>
      <div className="min-w-0 flex-1">
        {/* Kinds added on the server later show their code until they get a translation. */}
        <p className="font-bold leading-snug">
          {t(`security.log.types.${event.type}`, { defaultValue: event.type })}
        </p>
        <p className="text-sm leading-snug text-ink-soft">{where}</p>
        {at && (
          <time dateTime={at.toISOString()} className="text-xs font-bold text-ink-faint">
            {at.toLocaleString(i18n.language, { dateStyle: "medium", timeStyle: "short" })}
          </time>
        )}
      </div>
    </li>
  );
}

/** The latest few events, on the security screen. */
export function RecentSecurityEvents() {
  const { t } = useTranslation("settings");
  const { data, isPending } = useQuery({
    queryKey: [...keys.securityEvents, "recent"],
    queryFn: () => accountClient().listSecurityEvents({ page: { limit: 3 } }),
  });
  return (
    <Group title={t("security.log.title")} lead={t("security.log.lead")} flush>
      {isPending ? (
        <Skeleton className="m-1 h-14" />
      ) : (
        <ul>
          {data?.events.map((event) => (
            <EventRow key={event.id} event={event} />
          ))}
        </ul>
      )}
      <Link
        to="/settings/security/log"
        className="mt-1 flex items-center gap-2 rounded-2xl px-3 py-3 font-bold text-brand hover:bg-surface-2"
      >
        <span className="flex-1">{t("security.log.all")}</span>
        <ChevronRight className="size-5" />
      </Link>
    </Group>
  );
}

/** `/settings/security/log`: everything that happened to the account's sign-in, newest first. */
export function SecurityLogScreen() {
  const { t } = useTranslation("settings");
  const query = useInfiniteQuery({
    queryKey: [...keys.securityEvents, "all"],
    queryFn: ({ pageParam }) =>
      accountClient().listSecurityEvents({ page: { limit: 30, cursor: pageParam } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
  });
  const events = query.data?.pages.flatMap((page) => page.events) ?? [];

  return (
    <SettingsPage title={t("security.log.title")} back="/settings/security">
      <p className="px-1 pb-3 text-sm leading-relaxed text-ink-soft">{t("security.log.lead")}</p>
      {query.isPending ? (
        <div className="flex flex-col gap-2">
          {[0, 1, 2, 3].map((i) => (
            <Skeleton key={i} className="h-16" />
          ))}
        </div>
      ) : events.length === 0 ? (
        <EmptyState emoji="🧾" title={t("security.log.empty")} />
      ) : (
        <>
          <ul className="sticker rounded-card bg-surface p-2">
            {events.map((event) => (
              <EventRow key={event.id} event={event} />
            ))}
          </ul>
          <LoadMore query={query} showEnd />
        </>
      )}
    </SettingsPage>
  );
}
