import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { type notes, toMillis } from "@tokuly/api";
import { listKeys } from "@tokuly/core/notes";
import { MoreHorizontal, Pencil, Plus, Radio, Trash2 } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { Menu } from "#/components/ui/menu";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";
import { AntennaEditor, sourceKey } from "./antennas-editor";
import { ANTENNA_LIMITS, antennaClient, useAntenna, useAntennas } from "./lists-api";
import { LISTS_NS } from "./lists-editor";
import { ComposeButton, NotesPage } from "./shell";
import { noteAge } from "./time";
import { Timeline, type TimelineHandle } from "./timeline";

/** What an antenna listens for, in one line. */
function useAntennaSummary() {
  const { t, i18n } = useTranslation("notelists");
  return (antenna: notes.Antenna) => {
    const words = antenna.keywords.map((group) => group.words.join(" + ")).join(" / ");
    const caught = antenna.lastMatchedAt
      ? t("antennas.lastMatched", { time: noteAge(toMillis(antenna.lastMatchedAt), i18n.language) })
      : t("antennas.never");
    return {
      words: words || t("antennas.noKeywords"),
      details: [t(`antennas.source.${sourceKey(antenna.source)}`), caught].join(" · "),
    };
  };
}

/** `/notes/antennas`: the viewer's antennas. */
export function AntennasScreen() {
  const { t } = useTranslation("notelists");
  const antennas = useAntennas();
  const summary = useAntennaSummary();
  const [creating, setCreating] = useState(false);
  const full = (antennas.data?.length ?? 0) >= ANTENNA_LIMITS.antennas;
  const create = (
    <Button icon={<Radio className="size-4" />} onClick={() => setCreating(true)}>
      {t("antennas.create")}
    </Button>
  );
  return (
    <NotesPage
      title={t("antennas.title")}
      back="/notes"
      scrollId="note-antennas"
      actions={
        <Button
          size="sm"
          icon={<Plus className="size-4" />}
          disabled={full}
          onClick={() => setCreating(true)}
        >
          {t("antennas.new")}
        </Button>
      }
    >
      <p className="mb-3 px-1 text-sm text-ink-soft">
        {t("antennas.lead", { count: ANTENNA_LIMITS.antennas })}
      </p>
      {antennas.isPending ? (
        <div className="flex flex-col gap-2" aria-busy>
          {[0, 1].map((index) => (
            <Skeleton key={index} className="h-[4.5rem] rounded-card" />
          ))}
        </div>
      ) : antennas.isError ? (
        <EmptyState emoji="😵" title={errorMessage(antennas.error, LISTS_NS)} />
      ) : antennas.data.length === 0 ? (
        <EmptyState
          emoji="📡"
          title={t("antennas.empty")}
          body={t("antennas.emptyBody")}
          action={create}
        />
      ) : (
        <div className="flex flex-col gap-2">
          {antennas.data.map((antenna) => {
            const line = summary(antenna);
            return (
              <Link
                key={antenna.id}
                to="/notes/antennas/$antennaId"
                params={{ antennaId: antenna.id }}
                data-testid="antenna"
                className="pressable sticker-sm flex items-center gap-3 rounded-card bg-surface p-3.5"
              >
                <span className="grid size-11 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sun text-[#17122b]">
                  <Radio className="size-5" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-black">{antenna.name}</span>
                  <span className="block truncate text-sm font-bold">{line.words}</span>
                  <span className="block truncate text-xs text-ink-soft">{line.details}</span>
                </span>
              </Link>
            );
          })}
        </div>
      )}
      <AntennaEditor open={creating} onOpenChange={setCreating} />
    </NotesPage>
  );
}

/** `/notes/antennas/<id>`: what one antenna collected, newest first. */
export function AntennaScreen({ antennaId }: { antennaId: string }) {
  const { t } = useTranslation("notelists");
  const timeline = useRef<TimelineHandle>(null);
  const query = useAntenna(antennaId);
  const summary = useAntennaSummary();
  const antenna = query.data;
  if (query.isError || antenna === null) {
    return (
      <NotesPage
        title={t("antennas.notFound")}
        back="/notes/antennas"
        scrollId={`antenna-${antennaId}`}
      >
        <EmptyState
          emoji="📡"
          title={query.isError ? errorMessage(query.error, LISTS_NS) : t("antennas.notFound")}
        />
      </NotesPage>
    );
  }
  const line = antenna && summary(antenna);
  return (
    <NotesPage
      title={antenna?.name ?? t("antennas.title")}
      back="/notes/antennas"
      scrollId={`antenna-${antennaId}`}
      onRefresh={() => timeline.current?.refresh() ?? Promise.resolve()}
      actions={antenna && <AntennaMenu antenna={antenna} />}
    >
      {antenna && line && (
        <div className="sticker mb-3 flex items-center gap-3 rounded-card bg-surface p-4">
          <span className="grid size-12 shrink-0 place-items-center rounded-2xl border-2 border-ink bg-sun text-[#17122b]">
            <Radio className="size-6" />
          </span>
          <div className="min-w-0 flex-1">
            <p className="truncate font-black" data-testid="antenna-words">
              {line.words}
            </p>
            <p className="truncate text-sm text-ink-soft">{line.details}</p>
          </div>
        </div>
      )}
      <Timeline
        ref={timeline}
        source={{ kind: "antenna", antennaId }}
        empty={
          <EmptyState
            emoji="📡"
            title={t("antennas.emptyTimeline")}
            body={t("antennas.emptyTimelineBody")}
          />
        }
      />
      <ComposeButton />
    </NotesPage>
  );
}

function AntennaMenu({ antenna }: { antenna: notes.Antenna }) {
  const { t } = useTranslation("notelists");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [editing, setEditing] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const remove = useMutation({
    mutationFn: () => antennaClient().deleteAntenna({ antennaId: antenna.id }),
    onSuccess: async () => {
      setDeleting(false);
      toast.success(t("toast.antennaDeleted"));
      queryClient.removeQueries({ queryKey: listKeys.antenna(antenna.id) });
      await queryClient.invalidateQueries({ queryKey: listKeys.antennas, exact: true });
      void navigate({ to: "/notes/antennas" });
    },
    onError: (error) => toast.error(errorMessage(error, LISTS_NS)),
  });
  return (
    <>
      <Menu
        trigger={
          <Button size="icon-sm" variant="ghost" aria-label={t("antennas.edit")}>
            <MoreHorizontal className="size-5" />
          </Button>
        }
        actions={[
          {
            label: t("antennas.edit"),
            icon: <Pencil className="size-4" />,
            onSelect: () => setEditing(true),
          },
          {
            label: t("antennas.delete"),
            icon: <Trash2 className="size-4" />,
            danger: true,
            onSelect: () => setDeleting(true),
          },
        ]}
      />
      <AntennaEditor open={editing} onOpenChange={setEditing} antenna={antenna} />
      <ConfirmDialog
        open={deleting}
        onOpenChange={setDeleting}
        title={t("antennas.deleteConfirm.title", { name: antenna.name })}
        description={t("antennas.deleteConfirm.body")}
        confirmLabel={t("antennas.deleteConfirm.confirm")}
        danger
        loading={remove.isPending}
        onConfirm={() => remove.mutate()}
      />
    </>
  );
}
