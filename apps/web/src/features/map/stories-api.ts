import { useQuery } from "@tanstack/react-query";
import { feed } from "@tokuly/api";
import { mapKeys } from "@tokuly/core/map";
import { api } from "#/lib/api";

export const mapStoryClient = () => api().service(feed.MapStoryService);

export const useStoryTray = () =>
  useQuery({
    queryKey: mapKeys.storyTray,
    queryFn: () => mapStoryClient().getMapStoryTray({}),
    staleTime: 60_000,
  });
