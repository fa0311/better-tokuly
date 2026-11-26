import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { spaces } from "@tokuly/api";
import { spacesKeys } from "@tokuly/core/spaces";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { EmptyState, Page, Skeleton } from "#/components/ui/page";
import { useSpace } from "../api";
import { ExitNotice } from "../exit-notice";
import { useSpaceTopic } from "../live";
import { acknowledgeExit, enterSpace, useAudioSession } from "../session";
import { LiveRoom } from "./live-room";
import { EndedRoom, ScheduledRoom } from "./other-states";

/** One room, whatever state it is in: announced, on, or over. */
export function SpaceScreen({ spaceId }: { spaceId: string }) {
  const { t } = useTranslation("spaces");
  const queryClient = useQueryClient();
  const query = useSpace(spaceId);
  const space = query.data;
  // The topic is only open to people who may see the room.
  useSpaceTopic(space ? space.id : undefined);
  const exit = useAudioSession((state) =>
    state.exit?.kind === "space" && state.exit.id === spaceId ? state.exit : null,
  );

  // A room that ended says so itself; every other way of being put outside gets its own page.
  if (exit && exit.reason !== "ended") {
    const comeBack = exit.reason !== "banned" && space?.state === spaces.SpaceState.LIVE;
    return (
      <ExitNotice
        reason={exit.reason}
        action={
          comeBack && (
            <Button
              onClick={() => {
                acknowledgeExit();
                void queryClient.invalidateQueries({ queryKey: spacesKeys.space(spaceId) });
                void enterSpace(queryClient, space);
              }}
            >
              {t(exit.reason === "removed" ? "exit.enterAgain" : "exit.reconnect")}
            </Button>
          )
        }
      />
    );
  }
  if (query.isPending)
    return (
      <Page title={t("title")} back="/spaces">
        <Skeleton className="h-64" />
      </Page>
    );
  if (!space)
    return (
      <Page title={t("title")} back="/spaces">
        <EmptyState
          emoji="🫥"
          title={t("room.notFound")}
          body={t("room.notFoundBody")}
          action={
            <Link
              to="/spaces"
              className="pressable sticker inline-flex h-11 items-center rounded-full bg-brand px-5 text-[15px] font-bold text-on-brand"
            >
              {t("exit.toList")}
            </Link>
          }
        />
      </Page>
    );
  if (space.state === spaces.SpaceState.SCHEDULED) return <ScheduledRoom space={space} />;
  if (space.state === spaces.SpaceState.LIVE) return <LiveRoom space={space} />;
  return <EndedRoom space={space} />;
}
