import { useMutation, useQueryClient } from "@tanstack/react-query";
import { plans } from "@tokuly/api";
import { canWork, isHostLike, mergeChecklist } from "@tokuly/core/plans";
import {
  ArrowLeftRight,
  Check,
  Hand,
  MoreHorizontal,
  Pencil,
  Plus,
  Trash2,
  Undo2,
} from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Avatar } from "#/components/ui/avatar";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { Menu, type MenuAction } from "#/components/ui/menu";
import { Tabs } from "#/components/ui/tabs";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useMe } from "#/stores/session";
import { keys, nameOf, planClient } from "../api";
import { SectionTitle } from "../parts";

type Kind = "bring" | "todo";
const KINDS: Record<Kind, plans.ChecklistKind> = {
  bring: plans.ChecklistKind.BRING,
  todo: plans.ChecklistKind.TODO,
};

/**
 * 持ち物 and やること: what to bring and what to do, who takes care of each
 * item, and what is done. Ticks and claims show at once.
 */
export function Checklist({ plan }: { plan: plans.Plan }) {
  const { t } = useTranslation("plans");
  const me = useMe();
  const queryClient = useQueryClient();
  const [kind, setKind] = useState<Kind>("bring");
  const [text, setText] = useState("");
  const [editing, setEditing] = useState<{ item: plans.ChecklistItem; text: string }>();
  const allowed = canWork(plan);
  const host = isHostLike(plan);
  const myId = me?.user?.id ?? "";
  const people = new Map(
    plan.participants.map((participant) => [participant.user?.id ?? "", participant.user]),
  );

  const patch = (item: plans.ChecklistItem | undefined, removedId = "") =>
    queryClient.setQueryData<plans.Plan | null>(
      keys.plan(plan.id),
      (current) =>
        current && { ...current, checklist: mergeChecklist(current.checklist, item, removedId) },
    );
  const onError = (error: unknown) => {
    toast.error(errorMessage(error, { namespace: "plans" }));
    void queryClient.invalidateQueries({ queryKey: keys.plan(plan.id) });
  };

  const add = useMutation({
    mutationFn: () =>
      planClient().addChecklistItem({ planId: plan.id, kind: KINDS[kind], text: text.trim() }),
    onSuccess: (response) => {
      patch(response.item);
      setText("");
    },
    onError,
  });
  const update = useMutation({
    mutationFn: ({
      item,
      ...change
    }: {
      item: plans.ChecklistItem;
      text?: string;
      kind?: plans.ChecklistKind;
      claimed?: boolean;
      done?: boolean;
    }) => planClient().updateChecklistItem({ itemId: item.id, ...change }),
    onMutate: ({ item, claimed, done }) => {
      // Ticks and claims are drawn before the server answers.
      if (done !== undefined) patch({ ...item, done, doneByUserId: done ? myId : "" });
      else if (claimed !== undefined) patch({ ...item, claimedByUserId: claimed ? myId : "" });
    },
    onSuccess: (response) => {
      patch(response.item);
      setEditing(undefined);
    },
    onError,
  });
  const remove = useMutation({
    mutationFn: (item: plans.ChecklistItem) =>
      planClient().removeChecklistItem({ itemId: item.id }),
    onMutate: (item) => patch(undefined, item.id),
    onError,
  });

  const items = plan.checklist.filter(
    (item) => (item.kind === plans.ChecklistKind.TODO ? "todo" : "bring") === kind,
  );
  const count = (which: Kind) =>
    plan.checklist.filter(
      (item) => (item.kind === plans.ChecklistKind.TODO ? "todo" : "bring") === which && !item.done,
    ).length;

  const actionsFor = (item: plans.ChecklistItem): MenuAction[] => {
    if (!allowed) return [];
    const mineOrHost = item.createdByUserId === myId || host;
    const actions: MenuAction[] = [];
    if (item.claimedByUserId && (item.claimedByUserId === myId || host))
      actions.push({
        label: t("checklist.release"),
        icon: <Undo2 />,
        onSelect: () => update.mutate({ item, claimed: false }),
      });
    if (mineOrHost) {
      actions.push(
        {
          label: t("checklist.edit"),
          icon: <Pencil />,
          onSelect: () => setEditing({ item, text: item.text }),
        },
        {
          label:
            item.kind === plans.ChecklistKind.TODO ? t("checklist.toBring") : t("checklist.toTodo"),
          icon: <ArrowLeftRight />,
          onSelect: () =>
            update.mutate({
              item,
              kind:
                item.kind === plans.ChecklistKind.TODO
                  ? plans.ChecklistKind.BRING
                  : plans.ChecklistKind.TODO,
            }),
        },
        {
          label: t("checklist.delete"),
          icon: <Trash2 />,
          danger: true,
          onSelect: () => remove.mutate(item),
        },
      );
    }
    return actions;
  };

  return (
    <section>
      <SectionTitle>{t("checklist.title")}</SectionTitle>
      <div className="rounded-card bg-surface p-3">
        <Tabs<Kind>
          value={kind}
          onChange={setKind}
          items={[
            { value: "bring", label: t("checklist.bring"), badge: count("bring") },
            { value: "todo", label: t("checklist.todo"), badge: count("todo") },
          ]}
        />
        {items.length === 0 ? (
          <p className="px-2 py-6 text-center text-sm font-bold text-ink-soft">
            {kind === "bring" ? t("checklist.emptyBring") : t("checklist.emptyTodo")}
          </p>
        ) : (
          <ul className="mt-2 flex flex-col">
            {items.map((item) => {
              const claimer = item.claimedByUserId ? people.get(item.claimedByUserId) : undefined;
              const actions = actionsFor(item);
              return (
                <li
                  key={item.id}
                  className="flex items-center gap-2 rounded-2xl py-1.5 pr-1 pl-1"
                  data-testid="checklist-item"
                >
                  {/* biome-ignore lint/a11y/useSemanticElements: a large tick box drawn as a button. */}
                  <button
                    type="button"
                    role="checkbox"
                    aria-checked={item.done}
                    aria-label={t("checklist.tick", { text: item.text })}
                    disabled={!allowed}
                    onClick={() => update.mutate({ item, done: !item.done })}
                    className={cn(
                      "grid size-8 shrink-0 place-items-center rounded-xl border-[2.5px] transition-colors disabled:opacity-60",
                      item.done
                        ? "border-ink bg-lime text-[#17122b]"
                        : "border-ink-faint bg-surface",
                    )}
                  >
                    {item.done && <Check className="size-5" strokeWidth={3.5} />}
                  </button>
                  <span
                    className={cn(
                      "min-w-0 flex-1 font-bold break-words",
                      item.done && "text-ink-faint line-through",
                    )}
                  >
                    {item.text}
                  </span>
                  {claimer ? (
                    <span
                      className="flex shrink-0 items-center gap-1.5 rounded-full bg-surface-2 py-1 pr-2.5 pl-1 text-xs font-black"
                      title={nameOf(claimer)}
                    >
                      <Avatar user={claimer} size="xs" />
                      <span className="max-w-20 truncate">
                        {item.claimedByUserId === myId ? t("checklist.mine") : nameOf(claimer)}
                      </span>
                    </span>
                  ) : (
                    allowed &&
                    !item.done && (
                      <Button
                        variant="secondary"
                        size="sm"
                        className="h-8 shrink-0 px-3 text-xs"
                        icon={<Hand className="size-3.5" />}
                        onClick={() => update.mutate({ item, claimed: true })}
                      >
                        {kind === "bring" ? t("checklist.claimBring") : t("checklist.claimTodo")}
                      </Button>
                    )
                  )}
                  {actions.length > 0 && (
                    <Menu
                      actions={actions}
                      trigger={
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          className="shrink-0"
                          aria-label={t("checklist.more", { text: item.text })}
                        >
                          <MoreHorizontal className="size-5" />
                        </Button>
                      }
                    />
                  )}
                </li>
              );
            })}
          </ul>
        )}
        {allowed ? (
          <form
            className="mt-2 flex items-end gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              if (text.trim()) add.mutate();
            }}
          >
            <TextField
              className="min-w-0 flex-1"
              aria-label={kind === "bring" ? t("checklist.addBring") : t("checklist.addTodo")}
              placeholder={kind === "bring" ? t("checklist.addBring") : t("checklist.addTodo")}
              value={text}
              maxLength={100}
              onChange={(event) => setText(event.target.value)}
            />
            <Button
              type="submit"
              size="icon"
              className="size-12 shrink-0"
              aria-label={t("checklist.add")}
              loading={add.isPending}
              disabled={!text.trim()}
            >
              {!add.isPending && <Plus className="size-5" />}
            </Button>
          </form>
        ) : (
          plan.status === plans.PlanStatus.ACTIVE && (
            <p className="mt-2 px-2 text-xs font-bold text-ink-faint">{t("checklist.goingOnly")}</p>
          )
        )}
      </div>

      <Dialog
        open={Boolean(editing)}
        onOpenChange={(open) => !open && setEditing(undefined)}
        title={t("checklist.edit")}
        footer={
          <Button
            loading={update.isPending}
            disabled={!editing?.text.trim()}
            onClick={() =>
              editing && update.mutate({ item: editing.item, text: editing.text.trim() })
            }
          >
            {t("checklist.save")}
          </Button>
        }
      >
        <TextField
          aria-label={t("checklist.edit")}
          value={editing?.text ?? ""}
          maxLength={100}
          onChange={(event) => editing && setEditing({ ...editing, text: event.target.value })}
        />
      </Dialog>
    </section>
  );
}
