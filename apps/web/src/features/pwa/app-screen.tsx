import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import {
  BellRing,
  ChevronRight,
  Download,
  PartyPopper,
  Signal,
  Smartphone,
  Sparkles,
  Trash2,
  Zap,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Page, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { useTranslation } from "#/lib/use-translation";
import {
  hasGuide,
  type InstallWay,
  openInstallGuide,
  promptInstall,
  useInstall,
  useInstallWay,
  useIsStandalone,
} from "./install";
import { isIos } from "./platform";
import {
  applyUpdate,
  checkForUpdate,
  clearOfflineData,
  pageIsBehind,
  useWorker,
} from "./service-worker";
import { formatBytes, offlineData, storageUse, usedPercent } from "./storage";

/** Everything this page reads about the device; refetched after an action that changes it. */
const pwaKeys = {
  all: ["pwa"] as const,
  push: ["pwa", "push"] as const,
  offline: ["pwa", "offline"] as const,
  storage: ["pwa", "storage"] as const,
};

/**
 * "アプリとして使う": installing Tokuly, and how it stands on this device —
 * installed or not, push, offline readiness, the version, and the room it
 * takes, with a way to free it.
 */
export function AppScreen() {
  const { t } = useTranslation("pwa");
  return (
    <Page wide title={t("title")} back="/settings">
      {/* One column on phones; wide screens put the state next to installing and storage. */}
      <div className="grid gap-x-8 gap-y-7 lg:grid-cols-2 lg:items-start">
        <div className="flex min-w-0 flex-col gap-7">
          <InstallCard />
          <StorageGroup />
        </div>
        <Group title={t("status.title")}>
          <InstallRow />
          <PushRow />
          <OfflineRow />
          <VersionRow />
        </Group>
      </div>
    </Page>
  );
}

// ── Installing ──────────────────────────────────────────────────────

function InstallCard() {
  const { t } = useTranslation("pwa");
  const way = useInstallWay();
  const justInstalled = useInstall((state) => state.justInstalled);
  const [busy, setBusy] = useState(false);

  if (way === "installed" || justInstalled) {
    return (
      <section className="sticker flex items-center gap-4 rounded-card bg-lime p-5 text-[#17122b]">
        <span className="sticker-sm grid size-14 shrink-0 place-items-center rounded-2xl bg-white">
          <PartyPopper className="size-7 animate-wiggle" />
        </span>
        <div className="min-w-0 flex-1">
          <h2 className="text-lg font-black leading-snug">
            {way === "installed" ? t("install.installedTitle") : t("install.justInstalledTitle")}
          </h2>
          <p className="mt-0.5 text-sm font-normal leading-relaxed">
            {way === "installed" ? t("install.installedBody") : t("install.justInstalledBody")}
          </p>
        </div>
      </section>
    );
  }

  const install = async () => {
    setBusy(true);
    await promptInstall();
    setBusy(false);
  };

  return (
    <section className="sticker rounded-card bg-surface p-5">
      <div className="flex items-center gap-4">
        <img src="/icon.svg" alt="" className="sticker-sm size-16 shrink-0 rounded-[22%]" />
        <h2 className="min-w-0 flex-1 text-balance text-xl font-black leading-snug [word-break:auto-phrase]">
          {t("install.title")}
        </h2>
      </div>
      <ul className="mt-4 flex flex-col gap-2.5">
        <Perk icon={<Zap />} tint="bg-sun">
          {t("install.perks.fast")}
        </Perk>
        <Perk icon={<BellRing />} tint="bg-pink">
          {t("install.perks.push")}
        </Perk>
        <Perk icon={<Signal />} tint="bg-sky">
          {t("install.perks.calm")}
        </Perk>
      </ul>
      {way === "prompt" ? (
        <Button
          size="lg"
          block
          className="mt-5"
          icon={<Download className="size-5" />}
          loading={busy}
          onClick={install}
        >
          {t("install.install")}
        </Button>
      ) : hasGuide(way) ? (
        <Button size="lg" block className="mt-5" onClick={openInstallGuide}>
          {t("install.howTo")}
        </Button>
      ) : (
        <p className="mt-5 rounded-2xl bg-surface-2 px-4 py-3 text-sm font-normal leading-relaxed text-ink-soft [word-break:auto-phrase]">
          {t(`install.hints.${hintKey(way)}`)}
        </p>
      )}
    </section>
  );
}

/** The hint for browsers that install from their own menus, or not at all. */
function hintKey(way: InstallWay): "menu" | "macDock" | "none" {
  if (way === "menu") return "menu";
  return way === "mac-dock" ? "macDock" : "none";
}

function Perk({ icon, tint, children }: { icon: ReactNode; tint: string; children: ReactNode }) {
  return (
    <li className="flex items-center gap-3 font-bold">
      <span
        className={cn(
          "grid size-8 shrink-0 place-items-center rounded-xl border-2 border-ink text-[#17122b] [&>svg]:size-4",
          tint,
        )}
      >
        {icon}
      </span>
      {children}
    </li>
  );
}

// ── State ───────────────────────────────────────────────────────────

function InstallRow() {
  const { t } = useTranslation("pwa");
  const installed = useIsStandalone();
  return (
    <Row
      icon={<Smartphone />}
      tint="bg-lime"
      label={t("status.install.label")}
      hint={installed ? t("status.install.yesHint") : t("status.install.noHint")}
      chip={
        <StateChip tone={installed ? "good" : "off"}>
          {installed ? t("status.install.yes") : t("status.install.no")}
        </StateChip>
      }
    />
  );
}

type PushState = "on" | "off" | "blocked" | "unsupported";

/** Whether this browser holds a push subscription it may use. The notification settings know the rest. */
async function pushState(): Promise<PushState> {
  if (!("serviceWorker" in navigator) || !("PushManager" in window) || !("Notification" in window))
    return "unsupported";
  if (Notification.permission === "denied") return "blocked";
  if (Notification.permission !== "granted") return "off";
  const registration = await navigator.serviceWorker.getRegistration();
  return (await registration?.pushManager.getSubscription()) ? "on" : "off";
}

function PushRow() {
  const { t } = useTranslation("pwa");
  const installed = useIsStandalone();
  const { data: state } = useQuery({ queryKey: pwaKeys.push, queryFn: pushState, staleTime: 0 });
  // iOS only has push for the installed app: say what would turn "not supported" around.
  const needsInstall =
    state === "unsupported" && !installed && isIos(navigator.userAgent, navigator.maxTouchPoints);
  return (
    <Link
      to="/notifications/settings"
      aria-label={t("status.push.open")}
      className="block rounded-2xl transition-colors hover:bg-surface-2"
    >
      <Row
        icon={<BellRing />}
        tint="bg-pink"
        label={t("status.push.label")}
        hint={
          state
            ? t(needsInstall ? "status.push.needsInstallHint" : `status.push.${state}Hint`)
            : undefined
        }
        chip={
          state ? (
            <StateChip tone={state === "on" ? "good" : state === "blocked" ? "warn" : "off"}>
              {t(`status.push.${state}`)}
            </StateChip>
          ) : (
            <Skeleton className="h-6 w-14 rounded-full" />
          )
        }
        trailing={<ChevronRight className="size-5 shrink-0 text-ink-faint" />}
      />
    </Link>
  );
}

function OfflineRow() {
  const { t } = useTranslation("pwa");
  const status = useWorker((state) => state.status);
  // The worker stores the offline page while it installs: look again once it runs.
  const { data } = useQuery({
    queryKey: [...pwaKeys.offline, status],
    queryFn: offlineData,
    staleTime: 0,
  });
  const state =
    status === "unsupported" || status === "failed"
      ? "unsupported"
      : status === "active" && data?.ready
        ? "ready"
        : "preparing";
  return (
    <Row
      icon={<Signal />}
      tint="bg-sky"
      label={t("status.offline.label")}
      hint={t(`status.offline.${state}Hint`)}
      chip={
        <StateChip tone={state === "ready" ? "good" : "off"}>
          {t(`status.offline.${state}`)}
        </StateChip>
      }
    />
  );
}

function VersionRow() {
  const { t } = useTranslation("pwa");
  const status = useWorker((state) => state.status);
  const updateReady = useWorker(pageIsBehind);
  const [checking, setChecking] = useState(false);

  const check = async () => {
    setChecking(true);
    const found = await checkForUpdate();
    setChecking(false);
    // A found update announces itself (this row and the toast).
    if (!found) toast.success(t("status.version.upToDate"));
  };

  return (
    <Row
      icon={<Sparkles />}
      tint="bg-sun"
      label={t("status.version.label")}
      hint={updateReady ? t("status.version.updateHint") : t("status.version.latestHint")}
      chip={
        <StateChip tone={updateReady ? "warn" : "good"}>
          {updateReady ? t("status.version.update") : t("status.version.latest")}
        </StateChip>
      }
      trailing={
        updateReady ? (
          <Button size="sm" onClick={applyUpdate}>
            {t("status.version.apply")}
          </Button>
        ) : (
          status === "active" && (
            <Button size="sm" variant="secondary" loading={checking} onClick={check}>
              {t("status.version.check")}
            </Button>
          )
        )
      }
    />
  );
}

// ── Storage ─────────────────────────────────────────────────────────

function StorageGroup() {
  const { t, i18n } = useTranslation("pwa");
  const queryClient = useQueryClient();
  const use = useQuery({ queryKey: pwaKeys.storage, queryFn: storageUse, staleTime: 0 });
  const offline = useQuery({ queryKey: pwaKeys.offline, queryFn: offlineData, staleTime: 0 });
  const [confirming, setConfirming] = useState(false);
  const [clearing, setClearing] = useState(false);

  const clear = async () => {
    setClearing(true);
    try {
      await clearOfflineData();
      toast.success(t("storage.cleared"));
    } catch {
      toast.error(t("storage.clearFailed"));
    }
    setClearing(false);
    setConfirming(false);
    void queryClient.invalidateQueries({ queryKey: pwaKeys.all });
  };

  return (
    <Group title={t("storage.title")} lead={t("storage.lead")} padded>
      {use.isPending ? (
        <Skeleton className="h-16" />
      ) : use.data ? (
        <>
          <p className="flex flex-wrap items-baseline gap-x-2">
            <span className="text-3xl font-black" data-testid="storage-used">
              {formatBytes(use.data.usedBytes, i18n.language)}
            </span>
            <span className="text-sm font-bold text-ink-soft">{t("storage.used")}</span>
          </p>
          <div
            role="progressbar"
            aria-label={t("storage.meter")}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(usedPercent(use.data))}
            className="mt-3 h-4 overflow-hidden rounded-full border-2 border-ink bg-surface-2"
          >
            <div
              className="h-full rounded-full bg-brand transition-[width] duration-500"
              style={{ width: `${usedPercent(use.data)}%` }}
            />
          </div>
          <p className="mt-2 text-sm text-ink-soft">
            {t("storage.of", { quota: formatBytes(use.data.quotaBytes, i18n.language) })}
          </p>
        </>
      ) : (
        <p className="text-sm leading-relaxed text-ink-soft">{t("storage.unknown")}</p>
      )}
      <p className="mt-4 text-sm font-bold" data-testid="offline-files">
        {t("storage.files", { count: offline.data?.files ?? 0 })}
      </p>
      <Button
        variant="secondary"
        block
        className="mt-4"
        icon={<Trash2 className="size-4" />}
        onClick={() => setConfirming(true)}
      >
        {t("storage.clear")}
      </Button>
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("storage.clearTitle")}
        description={t("storage.clearBody")}
        confirmLabel={t("storage.clearConfirm")}
        danger
        loading={clearing}
        onConfirm={clear}
      />
    </Group>
  );
}

// ── Pieces ──────────────────────────────────────────────────────────

/** A titled card of related things, as on the settings screens. */
function Group({
  title,
  lead,
  padded,
  children,
}: {
  title: string;
  lead?: string /** A card that pads its content; otherwise rows reach its edge. */;
  padded?: boolean;
  children: ReactNode;
}) {
  return (
    <section className="min-w-0">
      <h2 className="px-1 text-lg font-black">{title}</h2>
      {lead && <p className="mt-0.5 px-1 text-sm leading-relaxed text-ink-soft">{lead}</p>}
      <div className={cn("sticker mt-3 rounded-card bg-surface", padded ? "p-4" : "p-2")}>
        {children}
      </div>
    </section>
  );
}

/** One fact about the app on this device: what it is, how it stands, and (sometimes) what to do about it. */
function Row({
  icon,
  tint,
  label,
  hint,
  chip,
  trailing,
}: {
  icon: ReactNode;
  tint: string;
  label: string;
  hint?: string;
  chip: ReactNode;
  trailing?: ReactNode;
}) {
  return (
    <div className="flex items-center gap-3 px-3 py-3">
      <span
        className={cn(
          "grid size-10 shrink-0 place-items-center rounded-xl border-2 border-ink text-[#17122b] [&>svg]:size-5",
          tint,
        )}
      >
        {icon}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 font-black">
          {label}
          {chip}
        </div>
        {hint && (
          <p className="mt-0.5 text-sm leading-snug text-ink-soft [word-break:auto-phrase]">
            {hint}
          </p>
        )}
      </div>
      {trailing}
    </div>
  );
}

const TONES = {
  good: "bg-lime text-[#17122b]",
  warn: "bg-sun text-[#17122b]",
  off: "bg-surface-2 text-ink-soft",
} as const;

/** A small state label. The text carries the meaning, the colour only backs it up. */
function StateChip({ tone, children }: { tone: keyof typeof TONES; children: ReactNode }) {
  return (
    <span
      className={cn(
        "inline-flex h-6 shrink-0 items-center rounded-full px-2.5 text-xs font-black",
        TONES[tone],
      )}
    >
      {children}
    </span>
  );
}
