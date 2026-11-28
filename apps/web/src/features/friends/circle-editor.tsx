import { useMutation, useQueryClient } from "@tanstack/react-query";
import { social } from "@tokuly/api";
import { Check, SmilePlus, X } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextField } from "#/components/ui/field";
import { EmojiPicker } from "#/features/emoji";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { useTranslation } from "#/lib/use-translation";
import { CIRCLE_COLORS, circleClient, circleKeys, isCloseFriends } from "./follow-data";

const NAME_MAX = 30;

/**
 * Making a circle, or changing one: a name, one emoji and a colour.
 * Close friends keeps its name and star and only takes a colour.
 */
export function CircleEditor({
  open,
  onOpenChange,
  circle,
  onSaved,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  /** The circle to change; a new one when absent. */
  circle?: social.Circle;
  onSaved?(circle: social.Circle): void;
}) {
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={<EditorTitle circle={circle} />}
      description={<EditorLead />}
    >
      {/* Remounted on every opening, so it starts from the circle as it is now. */}
      {open && <EditorForm circle={circle} onDone={() => onOpenChange(false)} onSaved={onSaved} />}
    </Dialog>
  );
}

function EditorTitle({ circle }: { circle?: social.Circle }) {
  const { t } = useTranslation("circles");
  return <>{circle ? t("editor.editTitle") : t("editor.createTitle")}</>;
}

function EditorLead() {
  const { t } = useTranslation("circles");
  return <>{t("editor.lead")}</>;
}

function EditorForm({
  circle,
  onDone,
  onSaved,
}: {
  circle?: social.Circle;
  onDone(): void;
  onSaved?(circle: social.Circle): void;
}) {
  const { t } = useTranslation("circles");
  const queryClient = useQueryClient();
  const fixed = circle ? isCloseFriends(circle) : false;
  const [name, setName] = useState(circle?.name ?? "");
  const [emoji, setEmoji] = useState(circle?.emoji ?? "");
  const [color, setColor] = useState<social.CircleColor>(circle?.color || social.CircleColor.BRAND);

  const save = useMutation({
    mutationFn: async () => {
      if (!circle)
        return (await circleClient().createCircle({ name: name.trim(), emoji, color })).circle;
      return (
        await circleClient().updateCircle(
          fixed
            ? { circleId: circle.id, color }
            : { circleId: circle.id, name: name.trim(), emoji, color },
        )
      ).circle;
    },
    onSuccess: (saved) => {
      toast.success(circle ? t("toast.saved") : t("toast.created"));
      onDone();
      if (saved) onSaved?.(saved);
      return queryClient.invalidateQueries({ queryKey: circleKeys.all });
    },
    onError: (error) => toast.error(errorMessage(error, { namespace: "circles" })),
  });

  return (
    <form
      className="flex flex-col gap-5"
      onSubmit={(event) => {
        event.preventDefault();
        save.mutate();
      }}
    >
      {!fixed && (
        <div className="flex items-end gap-3">
          <div className="flex flex-col items-center gap-1">
            <EmojiPicker
              unicodeOnly
              side="bottom"
              onPick={(picked) => setEmoji(picked)}
              trigger={
                <button
                  type="button"
                  aria-label={t("editor.emoji")}
                  className={cn(
                    "pressable sticker grid size-12 place-items-center rounded-2xl text-2xl",
                    CIRCLE_COLORS.find((entry) => entry.color === color)?.bg,
                  )}
                >
                  {emoji || <SmilePlus className="size-6 text-[#17122b]" />}
                </button>
              }
            />
            {emoji && (
              <button
                type="button"
                className="text-xs font-bold text-ink-faint hover:text-ink"
                onClick={() => setEmoji("")}
                aria-label={t("editor.clearEmoji")}
              >
                <X className="size-4" />
              </button>
            )}
          </div>
          <TextField
            className="min-w-0 flex-1"
            label={t("editor.name")}
            value={name}
            maxLength={NAME_MAX}
            placeholder={t("editor.namePlaceholder")}
            onChange={(event) => setName(event.target.value)}
            hint={`${[...name].length}/${NAME_MAX}`}
            autoFocus
          />
        </div>
      )}
      <fieldset>
        <legend className="mb-2 text-sm font-bold">{t("editor.color")}</legend>
        <div className="flex flex-wrap gap-2">
          {CIRCLE_COLORS.map((entry) => (
            <button
              key={entry.key}
              type="button"
              aria-label={t(`colors.${entry.key}`)}
              aria-pressed={color === entry.color}
              onClick={() => setColor(entry.color)}
              className={cn(
                "pressable sticker-sm grid size-10 place-items-center rounded-full text-[#17122b]",
                entry.bg,
                color === entry.color && "ring-4 ring-ink/30",
              )}
            >
              {color === entry.color && <Check className="size-5" />}
            </button>
          ))}
        </div>
      </fieldset>
      <Button type="submit" block loading={save.isPending} disabled={!fixed && !name.trim()}>
        {circle ? t("editor.save") : t("editor.create")}
      </Button>
    </form>
  );
}
