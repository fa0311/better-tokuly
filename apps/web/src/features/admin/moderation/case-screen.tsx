import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { admin, toMillis } from "@tokuly/api";
import { ExternalLink, Hand, RotateCcw, ShieldAlert, UserMinus, UserPlus } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { TextArea } from "#/components/ui/field";
import { LoadMore } from "#/components/ui/load-more";
import { EmptyState, Skeleton } from "#/components/ui/page";
import { cn } from "#/lib/cn";
import { errorMessage } from "#/lib/errors";
import { compactAge } from "#/lib/format";
import { useMe } from "#/stores/session";
import { moderationClient } from "../api";
import { formatDateTime } from "../format";
import { categoryKey } from "../labels";
import { ActionDialog, NOTE_MAX } from "../ui/action-dialog";
import { ConsolePage, Fact, Section } from "../ui/console-page";
import { CopyableId } from "../ui/copy";
import { useIsExtraWide } from "../ui/hooks";
import { ErrorState } from "../ui/states";
import { ToneChip } from "../ui/tone";
import { UserCell } from "../ui/user";
import { UserPicker } from "../ui/user-picker";
import { ActionList } from "./action-list";
import {
  CaseStateChip,
  CategoryChips,
  ModerationRecord,
  SubjectTypeChip,
  useCaseTitle,
} from "./parts";
import { type QueueSearch, useCaseQueue } from "./queue";
import { type Resolution, ResolvePanel } from "./resolve-panel";

/** `/admin/moderation/:caseId`: one case; on wide screens the queue stays beside it. */
export function CaseScreen({ caseId, search }: { caseId: string; search: QueueSearch }) {
  const showRail = useIsExtraWide();
  return (
    <div className="flex h-full">
      {showRail && <QueueRail search={search} currentId={caseId} />}
      <div className="min-w-0 flex-1">
        <CaseDetail key={caseId} caseId={caseId} search={search} />
      </div>
    </div>
  );
}

/** The queue as a narrow list, to move from case to case without going back. */
function QueueRail({ search, currentId }: { search: QueueSearch; currentId: string }) {
  const { t } = useTranslation("admin");
  const queue = useCaseQueue(search);
  const caseTitle = useCaseTitle();
  const cases = queue.data?.pages.flatMap((page) => page.cases) ?? [];
  return (
    <aside
      aria-label={t("moderation.rail.label")}
      className="flex w-72 shrink-0 flex-col border-r-2 border-line bg-surface"
    >
      <p className="flex h-14 shrink-0 items-center border-b-2 border-line px-4 text-sm font-black">
        {t(search.status === "resolved" ? "moderation.rail.resolved" : "moderation.rail.open")}
      </p>
      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        {queue.isPending ? (
          <div className="flex flex-col gap-2">
            {[0, 1, 2, 3].map((i) => (
              <Skeleton key={i} className="h-14" />
            ))}
          </div>
        ) : cases.length === 0 ? (
          <p className="px-2 py-4 text-sm text-ink-faint">{t("moderation.rail.empty")}</p>
        ) : (
          <ul className="flex flex-col gap-1">
            {cases.map((item) => (
              <li key={item.id}>
                <Link
                  to="/admin/moderation/$caseId"
                  params={{ caseId: item.id }}
                  search={search}
                  aria-current={item.id === currentId ? "page" : undefined}
                  className={cn(
                    "block rounded-xl px-3 py-2 text-sm",
                    item.id === currentId ? "bg-brand-soft" : "hover:bg-surface-2",
                  )}
                >
                  <span className="line-clamp-1 font-bold">{caseTitle(item)}</span>
                  <span className="mt-0.5 flex items-center gap-2 text-xs text-ink-soft">
                    <span>{t("moderation.reportCount", { count: item.reportCount })}</span>
                    <span>{compactAge(toMillis(item.lastReportedAt))}</span>
                  </span>
                </Link>
              </li>
            ))}
          </ul>
        )}
        <LoadMore query={queue} />
      </div>
    </aside>
  );
}

function CaseDetail({ caseId, search }: { caseId: string; search: QueueSearch }) {
  const { t } = useTranslation("admin");
  const queryClient = useQueryClient();
  const caseTitle = useCaseTitle();
  const key = ["admin", "case", caseId];
  const detail = useQuery({ queryKey: key, queryFn: () => moderationClient().getCase({ caseId }) });

  /** Shows the server's new version of the case at once, then re-reads everything that lists it. */
  const applied = (moderationCase: admin.ModerationCase | undefined) => {
    if (moderationCase)
      queryClient.setQueryData<admin.GetCaseResponse>(key, (previous) =>
        previous ? { ...previous, moderationCase } : previous,
      );
    return queryClient.invalidateQueries({ queryKey: ["admin"] });
  };
  const onError = (error: unknown) => toast.error(errorMessage(error));

  const resolve = useMutation({
    mutationFn: (resolution: Resolution) =>
      moderationClient().resolveCase({
        caseId,
        ...resolution,
        suspensionSeconds: BigInt(resolution.suspensionSeconds),
      }),
    onSuccess: (response) => {
      toast.success(t("moderation.toast.resolved"));
      return applied(response.moderationCase);
    },
    onError,
  });

  const back = t("moderation.title");
  if (detail.isPending) {
    return (
      <ConsolePage title={back} back="/admin/moderation" backSearch={search}>
        <div className="flex flex-col gap-4" aria-busy>
          <Skeleton className="h-40" />
          <Skeleton className="h-56" />
        </div>
      </ConsolePage>
    );
  }
  const moderationCase = detail.data?.moderationCase;
  if (!detail.data || !moderationCase) {
    return (
      <ConsolePage title={back} back="/admin/moderation" backSearch={search}>
        {detail.isError ? (
          <ErrorState error={detail.error} onRetry={() => detail.refetch()} />
        ) : (
          <EmptyState emoji="🧭" title={t("errors.CASE_NOT_FOUND")} />
        )}
      </ConsolePage>
    );
  }
  const isOpen = moderationCase.status === admin.CaseStatus.OPEN;

  return (
    <ConsolePage
      title={caseTitle(moderationCase)}
      back="/admin/moderation"
      backSearch={search}
      actions={<CaseStateChip moderationCase={moderationCase} />}
    >
      <div className="@container">
        <div className="grid items-start gap-4 @4xl:grid-cols-[minmax(0,1fr)_22rem]">
          <div className="flex min-w-0 flex-col gap-4">
            {moderationCase.conflictOfInterest && (
              <ConflictNotice own={!moderationCase.ownerWithheld} />
            )}
            <SubjectCard moderationCase={moderationCase} ownerRecord={detail.data.ownerRecord} />
            {/* Who reported it, and what staff noted, is for whoever handles the case. */}
            {!moderationCase.conflictOfInterest && (
              <>
                <Reports
                  caseId={caseId}
                  total={moderationCase.reportCount}
                  latest={detail.data.reports}
                />
                <Notes
                  caseId={caseId}
                  notes={detail.data.notes}
                  onAdded={() => queryClient.invalidateQueries({ queryKey: key })}
                />
              </>
            )}
          </div>
          <div className="flex min-w-0 flex-col gap-4">
            {/* A case about the viewer is not theirs to take (the server refuses). */}
            {!(moderationCase.conflictOfInterest && !moderationCase.ownerWithheld) && (
              <Assignment moderationCase={moderationCase} onChanged={applied} />
            )}
            {isOpen && moderationCase.conflictOfInterest ? (
              // Involved in the post: the server refuses every resolution, so none is offered.
              <Section title={t("moderation.resolve.title")}>
                <p className="p-4 text-sm text-ink-soft" data-testid="resolve-conflict">
                  {t(
                    moderationCase.ownerWithheld
                      ? "moderation.conflict.cannotResolve"
                      : "moderation.conflict.ownCannotResolve",
                  )}
                </p>
              </Section>
            ) : isOpen ? (
              <ResolvePanel
                moderationCase={moderationCase}
                loading={resolve.isPending}
                onResolve={(resolution) => resolve.mutate(resolution)}
              />
            ) : (
              <ResolutionCard moderationCase={moderationCase} onChanged={applied} />
            )}
            <Section title={t("moderation.history.title")}>
              <ActionList
                actions={detail.data.actions}
                showTarget
                empty={t("moderation.history.empty")}
              />
            </Section>
          </div>
        </div>
      </div>
    </ConsolePage>
  );
}

/**
 * Shown to a moderator who may not handle the case: it is about them or their own content (`own`), or
 * they are involved in an anonymous post whose author stays hidden from them.
 */
function ConflictNotice({ own }: { own: boolean }) {
  const { t } = useTranslation("admin");
  return (
    <div
      role="note"
      data-testid="case-conflict"
      className="flex gap-3 rounded-2xl border-2 border-ink bg-sun p-4 text-[#17122b]"
    >
      <ShieldAlert className="mt-0.5 size-5 shrink-0" />
      <div className="flex min-w-0 flex-col gap-1">
        <p className="font-black">{t("moderation.conflict.title")}</p>
        <p className="text-sm">
          {t(own ? "moderation.conflict.ownBody" : "moderation.conflict.body")}
        </p>
      </div>
    </div>
  );
}

function SubjectCard({
  moderationCase,
  ownerRecord,
}: {
  moderationCase: admin.ModerationCase;
  ownerRecord?: admin.UserModerationSummary;
}) {
  const { t } = useTranslation("admin");
  const caseTitle = useCaseTitle();
  return (
    <Section label={t("moderation.columns.subject")}>
      <div className="flex flex-col gap-3 p-4">
        <div className="flex flex-wrap items-center gap-2">
          <SubjectTypeChip type={moderationCase.subject?.type} />
          <CategoryChips categories={moderationCase.categories} />
          <span className="text-xs font-bold text-ink-soft">
            {t("moderation.reportCount", { count: moderationCase.reportCount })}
          </span>
        </div>
        <p className="break-words text-xl leading-snug font-black">{caseTitle(moderationCase)}</p>
        {moderationCase.subjectPath && (
          <a
            href={moderationCase.subjectPath}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1.5 self-start text-sm font-bold text-brand hover:underline"
          >
            <ExternalLink className="size-4" />
            {t("moderation.case.openInApp")}
            <span className="max-w-48 truncate font-mono text-xs font-normal text-ink-soft">
              {moderationCase.subjectPath}
            </span>
          </a>
        )}
      </div>
      <dl className="divide-y divide-line border-t-2 border-line">
        <Fact label={t("moderation.case.subjectId")}>
          {moderationCase.subject?.id ? <CopyableId value={moderationCase.subject.id} /> : "—"}
        </Fact>
        <Fact label={t("moderation.columns.owner")}>
          {moderationCase.ownerWithheld ? (
            <span data-testid="owner-withheld" className="text-ink-soft">
              {t(
                moderationCase.conflictOfInterest
                  ? "moderation.conflict.ownerHidden"
                  : "moderation.ownerWithheld",
              )}
            </span>
          ) : (
            <UserCell user={moderationCase.subjectOwner} fallback={t("moderation.ownerGone")} />
          )}
        </Fact>
        {ownerRecord && (
          <Fact label={t("moderation.case.ownerRecord")}>
            <ModerationRecord record={ownerRecord} />
          </Fact>
        )}
        <Fact label={t("moderation.case.firstReported")}>
          {formatDateTime(moderationCase.createdAt)}
        </Fact>
        <Fact label={t("moderation.columns.lastReported")}>
          {formatDateTime(moderationCase.lastReportedAt)}
        </Fact>
      </dl>
    </Section>
  );
}

function Assignment({
  moderationCase,
  onChanged,
}: {
  moderationCase: admin.ModerationCase;
  onChanged(moderationCase: admin.ModerationCase | undefined): unknown;
}) {
  const { t } = useTranslation("admin");
  const me = useMe();
  const [picking, setPicking] = useState(false);
  const assign = useMutation({
    mutationFn: (input: { assigneeId?: string; unassign?: boolean }) =>
      moderationClient().assignCase({ caseId: moderationCase.id, ...input }),
    onSuccess: (response, input) => {
      setPicking(false);
      toast.success(
        t(input.unassign ? "moderation.toast.unassigned" : "moderation.toast.assigned"),
      );
      return onChanged(response.moderationCase);
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  const isOpen = moderationCase.status === admin.CaseStatus.OPEN;
  const mine = moderationCase.assignee?.id === me?.user?.id;

  return (
    <Section title={t("moderation.assignment.title")}>
      <div className="flex flex-col gap-3 p-4">
        <div className="flex items-center gap-2 text-sm">
          <UserCell user={moderationCase.assignee} fallback={t("moderation.unassigned")} />
          {moderationCase.assignedAt && (
            <span className="text-xs text-ink-faint">
              {compactAge(toMillis(moderationCase.assignedAt))}
            </span>
          )}
        </div>
        {isOpen && (
          <div className="flex flex-wrap gap-2">
            {!mine && (
              <Button
                size="sm"
                variant="lime"
                icon={<Hand className="size-4" />}
                loading={assign.isPending && !picking}
                onClick={() => assign.mutate({})}
              >
                {t("moderation.assignment.takeIt")}
              </Button>
            )}
            <Button
              size="sm"
              variant="secondary"
              icon={<UserPlus className="size-4" />}
              onClick={() => setPicking((value) => !value)}
            >
              {t("moderation.assignment.someone")}
            </Button>
            {moderationCase.assignee && (
              <Button
                size="sm"
                variant="ghost"
                icon={<UserMinus className="size-4" />}
                onClick={() => assign.mutate({ unassign: true })}
              >
                {t("moderation.assignment.clear")}
              </Button>
            )}
          </div>
        )}
        {isOpen && picking && (
          <UserPicker
            label={t("moderation.assignment.pick")}
            staffOnly
            excludeIds={moderationCase.assignee ? [moderationCase.assignee.id] : undefined}
            onPick={(account) => assign.mutate({ assigneeId: account.user?.id })}
          />
        )}
      </div>
    </Section>
  );
}

/** How a closed case ended, with the way back into the queue. */
function ResolutionCard({
  moderationCase,
  onChanged,
}: {
  moderationCase: admin.ModerationCase;
  onChanged(moderationCase: admin.ModerationCase | undefined): unknown;
}) {
  const { t } = useTranslation("admin");
  const [reopening, setReopening] = useState(false);
  const reopen = useMutation({
    mutationFn: (note: string) =>
      moderationClient().reopenCase({ caseId: moderationCase.id, note }),
    onSuccess: (response) => {
      setReopening(false);
      toast.success(t("moderation.toast.reopened"));
      return onChanged(response.moderationCase);
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  return (
    <Section title={t("moderation.resolution.title")}>
      <dl className="divide-y divide-line">
        <Fact label={t("moderation.resolution.outcome")}>
          <CaseStateChip moderationCase={moderationCase} />
        </Fact>
        <Fact label={t("moderation.resolution.content")}>
          {t(
            moderationCase.contentRemoved
              ? "moderation.resolution.removed"
              : "moderation.resolution.kept",
          )}
        </Fact>
        {moderationCase.resolutionReason && (
          <Fact label={t("common.reasonShort")}>
            <span className="whitespace-pre-wrap">{moderationCase.resolutionReason}</span>
          </Fact>
        )}
        <Fact label={t("moderation.resolution.by")}>
          <UserCell user={moderationCase.resolvedBy} fallback={t("common.system")} />
        </Fact>
        <Fact label={t("moderation.resolution.at")}>
          {formatDateTime(moderationCase.resolvedAt)}
        </Fact>
      </dl>
      <div className="border-t-2 border-line p-4">
        <Button
          size="sm"
          variant="secondary"
          icon={<RotateCcw className="size-4" />}
          onClick={() => setReopening(true)}
        >
          {t("moderation.reopen.open")}
        </Button>
        <p className="mt-2 text-xs text-ink-faint">{t("moderation.reopen.hint")}</p>
      </div>
      <ActionDialog
        open={reopening}
        onOpenChange={setReopening}
        title={t("moderation.reopen.title")}
        description={t("moderation.reopen.description")}
        confirmLabel={t("moderation.reopen.confirm")}
        loading={reopen.isPending}
        noteLabel={t("moderation.reopen.note")}
        onConfirm={(texts) => reopen.mutate(texts.note)}
      />
    </Section>
  );
}

/** How many reports a case shows before "show all": enough to judge, short enough to keep the actions close. */
const REPORTS_SHOWN = 5;

/** The newest few reports from the case itself; "show all" pages through every one of them. */
function Reports({
  caseId,
  total,
  latest,
}: {
  caseId: string;
  total: number;
  latest: admin.CaseReport[];
}) {
  const { t } = useTranslation("admin");
  const [showAll, setShowAll] = useState(false);
  const all = useInfiniteQuery({
    queryKey: ["admin", "case", caseId, "reports"],
    queryFn: ({ pageParam }) =>
      moderationClient().listCaseReports({ caseId, page: { cursor: pageParam, limit: 50 } }),
    initialPageParam: "",
    getNextPageParam: (last) => last.page?.nextCursor || undefined,
    enabled: showAll,
  });
  // Until the full list arrives, "show all" shows what the case came with.
  const reports = !showAll
    ? latest.slice(0, REPORTS_SHOWN)
    : all.data
      ? all.data.pages.flatMap((page) => page.reports)
      : latest;

  return (
    <Section title={t("moderation.reports.title", { count: total })}>
      {reports.length === 0 ? (
        <p className="px-4 py-4 text-sm text-ink-faint">{t("moderation.reports.empty")}</p>
      ) : (
        <ul className="divide-y divide-line">
          {reports.map((report) => (
            <li key={report.id} className="flex flex-col gap-1.5 px-4 py-3 text-sm">
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                <UserCell user={report.reporter} fallback={t("moderation.reports.reporterGone")} />
                <ToneChip>{t(categoryKey(report.category))}</ToneChip>
                <span className="text-xs text-ink-soft">{formatDateTime(report.createdAt)}</span>
              </div>
              {report.text ? (
                <p className="whitespace-pre-wrap break-words">{report.text}</p>
              ) : (
                <p className="text-ink-faint">{t("moderation.reports.noText")}</p>
              )}
            </li>
          ))}
        </ul>
      )}
      {showAll ? (
        <LoadMore query={all} />
      ) : (
        total > REPORTS_SHOWN && (
          <div className="border-t-2 border-line p-3">
            <Button size="sm" variant="ghost" block onClick={() => setShowAll(true)}>
              {t("moderation.reports.showAll", { count: total })}
            </Button>
          </div>
        )
      )}
    </Section>
  );
}

function Notes({
  caseId,
  notes,
  onAdded,
}: {
  caseId: string;
  notes: admin.CaseNote[];
  onAdded(): unknown;
}) {
  const { t } = useTranslation("admin");
  const [body, setBody] = useState("");
  const add = useMutation({
    mutationFn: () => moderationClient().addCaseNote({ caseId, body: body.trim() }),
    onSuccess: () => {
      setBody("");
      toast.success(t("moderation.toast.noteAdded"));
      return onAdded();
    },
    onError: (error) => toast.error(errorMessage(error)),
  });
  return (
    <Section
      title={t("moderation.notes.title")}
      aside={<span className="text-xs text-ink-faint">{t("moderation.notes.internal")}</span>}
    >
      {notes.length > 0 && (
        <ul className="divide-y divide-line">
          {notes.map((note) => (
            <li key={note.id} className="flex flex-col gap-1.5 px-4 py-3 text-sm">
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                <UserCell user={note.author} fallback={t("common.system")} />
                <span className="text-xs text-ink-soft">{formatDateTime(note.createdAt)}</span>
              </div>
              <p className="whitespace-pre-wrap break-words">{note.body}</p>
            </li>
          ))}
        </ul>
      )}
      <form
        className={cn("flex flex-col gap-2 p-4", notes.length > 0 && "border-t-2 border-line")}
        onSubmit={(event) => {
          event.preventDefault();
          if (body.trim()) add.mutate();
        }}
      >
        <TextArea
          aria-label={t("moderation.notes.add")}
          placeholder={t("moderation.notes.placeholder")}
          value={body}
          maxLength={NOTE_MAX}
          onChange={(event) => setBody(event.target.value)}
        />
        <Button
          type="submit"
          size="sm"
          variant="secondary"
          className="self-end"
          disabled={!body.trim()}
          loading={add.isPending}
        >
          {t("moderation.notes.add")}
        </Button>
      </form>
    </Section>
  );
}
