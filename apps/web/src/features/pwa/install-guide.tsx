import {
  BookOpen,
  ChevronLeft,
  ChevronRight,
  Copy,
  Ellipsis,
  Link2,
  Share,
  SquarePlus,
} from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { cn } from "#/lib/cn";
import { closeInstallGuide, useInstall, useInstallWay } from "./install";

/**
 * How to add Tokuly to the home screen on an iPhone or iPad, where the
 * browser has no install prompt: three steps, each with a small picture of
 * what to look for. Opened with `openInstallGuide()`.
 */
export function InstallGuide() {
  const { t } = useTranslation("pwa");
  const open = useInstall((state) => state.guideOpen);
  const way = useInstallWay();

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => !next && closeInstallGuide()}
      title={t("guide.title")}
      description={t("guide.lead")}
      footer={
        <Button block onClick={closeInstallGuide}>
          {t("guide.done")}
        </Button>
      }
    >
      {way === "ios-in-app" && <OpenInSafari />}
      <ol className="flex flex-col gap-5">
        <Step
          number={1}
          title={t("guide.steps.share.title")}
          body={
            way === "ios-browser" ? t("guide.steps.share.browser") : t("guide.steps.share.safari")
          }
        >
          <ToolbarArt />
        </Step>
        <Step number={2} title={t("guide.steps.add.title")} body={t("guide.steps.add.body")}>
          <ShareSheetArt />
        </Step>
        <Step
          number={3}
          title={t("guide.steps.confirm.title")}
          body={t("guide.steps.confirm.body")}
        >
          <ConfirmArt />
        </Step>
      </ol>
    </Dialog>
  );
}

function Step({
  number,
  title,
  body,
  children,
}: {
  number: number;
  title: string;
  body: string;
  children: ReactNode;
}) {
  return (
    <li className="flex gap-3">
      <span
        aria-hidden
        className="sticker-sm grid size-8 shrink-0 place-items-center rounded-full bg-brand text-sm font-black text-on-brand"
      >
        {number}
      </span>
      <div className="min-w-0 flex-1">
        <p className="font-black leading-snug">{title}</p>
        <p className="mt-0.5 text-sm leading-relaxed text-ink-soft [word-break:auto-phrase]">
          {body}
        </p>
        <div aria-hidden className="mt-2.5 select-none rounded-2xl bg-surface-2 p-3">
          {children}
        </div>
      </div>
    </li>
  );
}

/** What to look for is drawn as a lime sticker; everything around it stays faint. */
const SPOT = "sticker-sm bg-lime text-[#17122b]";

/** Step 1: a browser bar with the Share button picked out. */
function ToolbarArt() {
  return (
    <div className="flex items-center justify-between rounded-full border-2 border-line bg-surface px-4 py-1.5 text-ink-faint">
      <ChevronLeft className="size-5" />
      <ChevronRight className="size-5" />
      <span className={cn("relative grid size-11 place-items-center rounded-full", SPOT)}>
        <span className="absolute inset-0 animate-pulse-ring rounded-full bg-lime" />
        <Share className="relative size-5" strokeWidth={2.5} />
      </span>
      <BookOpen className="size-5" />
      <Ellipsis className="size-5" />
    </div>
  );
}

/** Step 2: the share menu with "Add to Home Screen" picked out. */
function ShareSheetArt() {
  const { t } = useTranslation("pwa");
  const row = "flex items-center justify-between gap-3 px-3 py-2 text-sm font-bold";
  return (
    <div className="rounded-2xl border-2 border-line bg-surface p-1 text-ink-faint">
      <div className={row}>
        <span className="truncate">{t("guide.art.copy")}</span>
        <Copy className="size-4 shrink-0" />
      </div>
      <div className={cn(row, "rounded-xl font-black", SPOT)}>
        <span className="truncate">{t("guide.art.addToHome")}</span>
        <SquarePlus className="size-5 shrink-0" strokeWidth={2.5} />
      </div>
      <div className={row}>
        <span className="truncate">{t("guide.art.bookmark")}</span>
        <BookOpen className="size-4 shrink-0" />
      </div>
    </div>
  );
}

/** Step 3: the confirmation with "Add" picked out. */
function ConfirmArt() {
  const { t } = useTranslation("pwa");
  return (
    <div className="rounded-2xl border-2 border-line bg-surface p-3">
      <div className="flex items-center justify-between gap-2 text-sm font-bold">
        <span className="shrink-0 text-ink-faint">{t("guide.art.cancel")}</span>
        <span className="min-w-0 truncate text-ink-soft">{t("guide.art.addToHome")}</span>
        <span className={cn("shrink-0 rounded-full px-3 py-0.5 font-black", SPOT)}>
          {t("guide.art.add")}
        </span>
      </div>
      <div className="mt-3 flex items-center gap-3 rounded-xl bg-surface-2 p-2">
        <img src="/icon.svg" alt="" className="size-10 rounded-[22%]" />
        <span className="font-black">Tokuly</span>
      </div>
    </div>
  );
}

/** Inside another app's browser nothing can be added: the page has to move to Safari first. */
function OpenInSafari() {
  const { t } = useTranslation("pwa");
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(`${window.location.origin}/`);
      toast.success(t("guide.inApp.copied"));
    } catch {
      toast.error(t("guide.inApp.copyFailed"));
    }
  };
  return (
    <section className="sticker mb-5 rounded-card bg-sun p-4 text-[#17122b]">
      <p className="font-black leading-snug">{t("guide.inApp.title")}</p>
      <p className="mt-1 text-sm font-normal leading-relaxed">{t("guide.inApp.body")}</p>
      <Button
        variant="secondary"
        size="sm"
        className="mt-3"
        icon={<Link2 className="size-4" />}
        onClick={copy}
      >
        {t("guide.inApp.copy")}
      </Button>
    </section>
  );
}
