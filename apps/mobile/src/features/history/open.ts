import { router } from "expo-router";
import { webScreenHref } from "#/features/web/handoff";
import { screenFor } from "#/seams/links";

/** Opens a path of the server's (`/places/<id>`) where the app has it: its own screen, else the web one. */
export const openPath = (path: string) =>
  router.push((screenFor(path) ?? webScreenHref(path)) as never);
