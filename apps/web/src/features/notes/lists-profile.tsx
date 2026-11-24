import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { common } from "@tokuly/api";
import { Check, ListPlus, Plus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { EmptyState, ListRow } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { listClient, listKeys, resetListTimeline, storeList, useLists } from "./lists-api";
import { LISTS_NS, ListEditor } from "./lists-editor";
import { ListCard } from "./lists-screen";

/** "リストに追加" among a profile's actions: puts the person on one or more of the viewer's lists. */
export function AddToListButton({ user, isSelf }: { user: common.UserSummary; isSelf: boolean }) {
  const { t } = useTranslation("notelists");
  const [open, setOpen] = useState(false);
  if (isSelf || user.deleted) return null;
  const name = user.displayName || `@${user.handle}`;
  return (
    <>
      <Button
        variant="secondary"
        icon={<ListPlus className="size-4" />}
        onClick={() => setOpen(true)}
      >
        {t("profile.addToList")}
      </Button>
      <Dialog
        open={open}
        onOpenChange={setOpen}
        title={t("profile.addTitle", { name })}
        description={t("profile.addLead")}
      >
        {open && <ListChoices user={user} />}
      </Dialog>
    </>
  );
}

function ListChoices({ user }: { user: common.UserSummary }) {
  const { t } = useTranslation("notelists");
  const queryClient = useQueryClient();
  const lists = useLists();
  const [creating, setCreating] = useState(false);
  const [added, setAdded] = useState<ReadonlySet<string>>(new Set());
  const add = useMutation({
    mutationFn: (listId: string) => listClient().addListMembers({ listId, userIds: [user.id] }),
    onSuccess: async (response, listId) => {
      setAdded((current) => new Set(current).add(listId));
      storeList(queryClient, response.list);
      toast.success(t("toast.added", { name: user.displayName || `@${user.handle}` }));
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: listKeys.members(listId) }),
        resetListTimeline(queryClient, listId),
      ]);
    },
    onError: (error) => toast.error(errorMessage(error, LISTS_NS)),
  });
  if (lists.isPending) return null;
  const rows = lists.data ?? [];
  return (
    <div className="flex flex-col gap-1">
      {rows.length === 0 ? (
        <EmptyState
          emoji="📋"
          title={t("profile.noLists")}
          action={
            <Button icon={<Plus className="size-4" />} onClick={() => setCreating(true)}>
              {t("profile.createFirst")}
            </Button>
          }
        />
      ) : (
        rows.map((list) => (
          <ListRow
            key={list.id}
            title={list.name}
            subtitle={t("lists.members", { count: list.memberCount })}
            trailing={
              added.has(list.id) ? (
                <span className="inline-flex items-center gap-1 rounded-full bg-surface-2 px-3 py-1 text-xs font-bold text-ink-soft">
                  <Check className="size-3.5" aria-hidden />
                  {t("add.added")}
                </span>
              ) : (
                <Button
                  size="sm"
                  icon={<Plus className="size-4" />}
                  disabled={add.isPending}
                  aria-label={`${t("add.add")}: ${list.name}`}
                  onClick={() => add.mutate(list.id)}
                >
                  {t("add.add")}
                </Button>
              )
            }
          />
        ))
      )}
      <ListEditor open={creating} onOpenChange={setCreating} />
    </div>
  );
}

/** A person's public lists, near the bottom of their profile. Renders nothing while they have none. */
export function ProfileLists({ user }: { user: common.UserSummary }) {
  const { t } = useTranslation("notelists");
  const lists = useLists(user.id);
  const shown = (lists.data ?? []).filter((list) => list.public);
  if (shown.length === 0) return null;
  return (
    <section className="mt-6" data-testid="profile-lists">
      <h2 className="mb-2 px-1 text-sm font-black text-ink-soft">{t("profile.publicLists")}</h2>
      <div className="flex flex-col gap-2">
        {shown.map((list) => (
          <ListCard key={list.id} list={list} />
        ))}
      </div>
    </section>
  );
}
