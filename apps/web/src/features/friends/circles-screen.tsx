import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { social } from "@tokuly/api";
import { ArrowDown, ArrowUp, ChevronRight, Plus, Star, Trash2 } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { EmptyState, Page } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { CircleEditor } from "./circle-editor";
import { circleBg, circleClient, circleKeys, isCloseFriends } from "./follow-data";
import { RowsSkeleton, SectionTitle } from "./parts";

const NS = { namespace: "circles" };

export function useCircles() {
  return useQuery({
    queryKey: circleKeys.list,
    queryFn: async () => (await circleClient().listCircles({})).circles,
  });
}

/** The circle's bubble: the green star for close friends, its emoji (or initial) otherwise. */
export function CircleBadge({ circle, className }: { circle: social.Circle; className?: string }) {
  if (isCloseFriends(circle))
    return (
      <span
        className={cn(
          "sticker-sm grid size-11 shrink-0 place-items-center rounded-full bg-success text-white",
          className,
        )}
        aria-hidden
      >
        <Star className="size-6 fill-current" />
      </span>
    );
  return (
    <span
      className={cn(
        "sticker-sm grid size-11 shrink-0 place-items-center rounded-full text-xl font-black text-[#17122b]",
        circleBg(circle.color),
        className,
      )}
      aria-hidden
    >
      {circle.emoji || [...circle.name][0] || "•"}
    </span>
  );
}

/** What a circle is called: close friends has the app's own words for it. */
export function useCircleName() {
  const { t } = useTranslation("circles");
  return (circle: social.Circle) => (isCloseFriends(circle) ? t("closeFriends") : circle.name);
}

/** `/circles`: 親しい友達 first, then the viewer's own lists, which can be reordered and deleted. */
export function CirclesScreen() {
  const { t } = useTranslation("circles");
  const queryClient = useQueryClient();
  const circles = useCircles();
  const nameOf = useCircleName();
  const [creating, setCreating] = useState(false);
  const [deleting, setDeleting] = useState<social.Circle>();
  const [confirmOpen, setConfirmOpen] = useState(false);

  const closeFriends = circles.data?.find(isCloseFriends);
  const custom = circles.data?.filter((circle) => !isCloseFriends(circle)) ?? [];

  const reorder = useMutation({
    mutationFn: (ids: string[]) => circleClient().reorderCircles({ circleIds: ids }),
    onMutate: (ids) =>
      queryClient.setQueryData<social.Circle[]>(circleKeys.list, (data) =>
        data
          ? [
              ...data.filter(isCloseFriends),
              ...ids.flatMap((id) => data.filter((circle) => circle.id === id)),
            ]
          : data,
      ),
    onSuccess: (response) => queryClient.setQueryData(circleKeys.list, response.circles),
    onError: (error) => {
      toast.error(errorMessage(error, NS));
      return queryClient.invalidateQueries({ queryKey: circleKeys.list });
    },
  });
  const move = (index: number, by: -1 | 1) => {
    const ids = custom.map((circle) => circle.id);
    const [moved] = ids.splice(index, 1);
    ids.splice(index + by, 0, moved!);
    reorder.mutate(ids);
  };

  const remove = useMutation({
    mutationFn: (circle: social.Circle) => circleClient().deleteCircle({ circleId: circle.id }),
    onSuccess: (_, circle) => {
      setConfirmOpen(false);
      toast.success(t("toast.deleted", { name: circle.name }));
      return queryClient.invalidateQueries({ queryKey: circleKeys.all });
    },
    onError: (error) => {
      setConfirmOpen(false);
      toast.error(errorMessage(error, NS));
    },
  });

  return (
    <Page
      title={t("title")}
      back={{ fallback: "/friends" }}
      actions={
        <Button size="sm" icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
          {t("create")}
        </Button>
      }
    >
      <p className="mb-4 px-1 text-sm leading-relaxed text-ink-soft">{t("lead")}</p>
      {circles.isPending ? (
        <RowsSkeleton />
      ) : circles.isError ? (
        <EmptyState emoji="😵" title={errorMessage(circles.error, NS)} />
      ) : (
        <div className="flex flex-col gap-6">
          {closeFriends && (
            <Link
              to="/circles/$circleId"
              params={{ circleId: closeFriends.id }}
              data-testid="circle-row"
              className="pressable sticker flex items-center gap-3 rounded-card bg-surface p-3"
            >
              <CircleBadge circle={closeFriends} />
              <span className="min-w-0 flex-1">
                <span className="block font-black">{t("closeFriends")}</span>
                <span className="block text-sm text-ink-soft">
                  {t("members", { count: closeFriends.memberCount })}
                </span>
              </span>
              <ChevronRight className="size-5 text-ink-faint" aria-hidden />
            </Link>
          )}
          <section>
            <SectionTitle>{t("custom")}</SectionTitle>
            {custom.length === 0 ? (
              <EmptyState
                emoji="🫶"
                title={t("empty.title")}
                body={t("empty.body")}
                action={
                  <Button icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
                    {t("create")}
                  </Button>
                }
              />
            ) : (
              <ul className="flex flex-col gap-2">
                {custom.map((circle, index) => (
                  <li
                    key={circle.id}
                    data-testid="circle-row"
                    className="sticker-sm flex items-center gap-2 rounded-2xl bg-surface p-2 pl-3"
                  >
                    <Link
                      to="/circles/$circleId"
                      params={{ circleId: circle.id }}
                      className="flex min-w-0 flex-1 items-center gap-3"
                    >
                      <CircleBadge circle={circle} />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate font-black">{nameOf(circle)}</span>
                        <span className="block text-sm text-ink-soft">
                          {t("members", { count: circle.memberCount })}
                        </span>
                      </span>
                    </Link>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label={t("moveUp", { name: circle.name })}
                      disabled={index === 0 || reorder.isPending}
                      onClick={() => move(index, -1)}
                    >
                      <ArrowUp className="size-4" />
                    </Button>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label={t("moveDown", { name: circle.name })}
                      disabled={index === custom.length - 1 || reorder.isPending}
                      onClick={() => move(index, 1)}
                    >
                      <ArrowDown className="size-4" />
                    </Button>
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      aria-label={t("delete", { name: circle.name })}
                      onClick={() => {
                        setDeleting(circle);
                        setConfirmOpen(true);
                      }}
                    >
                      <Trash2 className="size-4" />
                    </Button>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>
      )}
      <CircleEditor open={creating} onOpenChange={setCreating} />
      <ConfirmDialog
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        title={t("deleteConfirm.title", { name: deleting?.name ?? "" })}
        description={t("deleteConfirm.body")}
        confirmLabel={t("deleteConfirm.confirm")}
        danger
        loading={remove.isPending}
        onConfirm={() => deleting && remove.mutate(deleting)}
      />
    </Page>
  );
}
