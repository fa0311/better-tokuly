import { Outlet, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { SETTINGS_SECTIONS } from "#/seams/settings";
import { IdentityCard, SettingsNav, SignOutButton } from "./settings-nav";

/**
 * Frame of every settings route. Wide screens get two panes: the section
 * list stays on the left while the open section scrolls on the right. On
 * phones only the route itself is shown (the list is the `/settings` page).
 */
export function SettingsLayout() {
  const { t } = useTranslation("settings");
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  // `/settings` itself shows the first section on wide screens.
  const activeId =
    SETTINGS_SECTIONS.find((section) => pathname.startsWith(section.to))?.id ?? "profile";

  return (
    <div className="flex h-full">
      <aside className="hidden h-full w-80 shrink-0 flex-col border-r-2 border-line lg:flex">
        <div className="flex h-16 shrink-0 items-center border-b-2 border-line px-5">
          <p className="text-xl font-black">{t("title")}</p>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-3">
          <IdentityCard />
          <SettingsNav activeId={activeId} />
          <div className="flex-1" />
          <SignOutButton />
        </div>
      </aside>
      <div className="h-full min-w-0 flex-1">
        <Outlet />
      </div>
    </div>
  );
}
