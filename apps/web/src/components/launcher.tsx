import { Search } from "lucide-react";
import { create } from "zustand";
import { HubSections } from "#/components/hub";
import { Dialog } from "#/components/ui/dialog";
import { openQuickSearch } from "#/features/search";
import { useTranslation } from "#/lib/use-translation";

const useLauncher = create<{ open: boolean }>(() => ({ open: false }));

/** Opens the launcher over whatever page is showing. */
export function openLauncher(): void {
  useLauncher.setState({ open: true });
}

export function useLauncherOpen(): boolean {
  return useLauncher((state) => state.open);
}

/**
 * The launcher: every part of the app in one sheet, without leaving the page
 * that is open. The rail's "アプリ" opens it; the hub page (`/me`) shows the
 * same tiles on phones.
 */
export function Launcher() {
  const { t } = useTranslation("common");
  const open = useLauncherOpen();
  const close = () => useLauncher.setState({ open: false });
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => useLauncher.setState({ open: next })}
      title={t("hub.launcher")}
      className="sm:w-[min(36rem,calc(100vw-2rem))]"
    >
      <button
        type="button"
        onClick={() => {
          close();
          openQuickSearch();
        }}
        className="mb-5 flex h-12 w-full items-center gap-3 rounded-full border-2 border-line bg-surface-2 px-4 text-left font-bold text-ink-soft transition-colors hover:border-ink"
      >
        <Search className="size-5 shrink-0" />
        <span className="min-w-0 flex-1 truncate">{t("hub.search")}</span>
      </button>
      <HubSections variant="launcher" onNavigate={close} />
    </Dialog>
  );
}
