import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Search, Trash2 } from "lucide-react";
import { type FormEvent, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { ConfirmDialog } from "#/components/ui/confirm";
import { TextField } from "#/components/ui/field";
import { dropArticle, newsAdminClient, newsClient, newsKeys } from "../api";
import { PublishedAt, SourceIcon } from "../parts";
import { articleIdFrom } from "./input";
import { Pane, staffErrorMessage } from "./parts";

/**
 * Takes one article out of every list and out of search; it is not fetched
 * again. Staff paste the article's address (or ID), check that it is the
 * right one, then confirm.
 */
export function RemoveArticle() {
  const { t } = useTranslation("news");
  const queryClient = useQueryClient();
  const [text, setText] = useState("");
  const [articleId, setArticleId] = useState<string>();
  const [confirming, setConfirming] = useState(false);
  const found = useQuery({
    queryKey: newsKeys.article(articleId ?? ""),
    queryFn: () => newsClient().getNewsArticle({ articleId: articleId as string }),
    enabled: Boolean(articleId),
    retry: false,
  });
  const remove = useMutation({
    mutationFn: (id: string) => newsAdminClient().removeNewsArticle({ articleId: id }),
    onSuccess: (_response, id) => {
      dropArticle(queryClient, id);
      toast.success(t("staff.articleRemoved"));
      setText("");
      setArticleId(undefined);
    },
    onError: (error) => toast.error(staffErrorMessage(error)),
    onSettled: () => setConfirming(false),
  });
  const pasted = articleIdFrom(text);
  const lookUp = (event: FormEvent) => {
    event.preventDefault();
    setArticleId(pasted);
  };
  const article = articleId ? found.data?.article : undefined;

  return (
    <Pane title={t("staff.remove.title")}>
      <form onSubmit={lookUp} className="flex flex-col gap-3 p-4">
        <p className="text-sm leading-relaxed text-ink-soft">{t("staff.remove.lead")}</p>
        <TextField
          label={t("staff.remove.field")}
          value={text}
          onChange={(event) => {
            setText(event.target.value);
            setArticleId(undefined);
          }}
          error={
            text.trim() && !pasted
              ? t("staff.remove.invalid")
              : articleId && found.isError
                ? staffErrorMessage(found.error)
                : undefined
          }
          placeholder="/news/01a200b9-…"
          autoCapitalize="none"
          autoComplete="off"
          spellCheck={false}
        />
        <Button
          type="submit"
          variant="secondary"
          disabled={!pasted}
          loading={Boolean(articleId) && found.isFetching}
          icon={<Search className="size-4" />}
        >
          {t("staff.remove.find")}
        </Button>
      </form>
      {article && (
        <div
          className="flex flex-col gap-3 border-t-2 border-line p-4"
          data-testid="article-to-remove"
        >
          <div>
            <p className="leading-snug font-black break-words">{article.title}</p>
            <p className="mt-1 flex min-w-0 items-center gap-1.5 text-xs text-ink-soft">
              <SourceIcon source={article.source} />
              <span className="truncate font-bold">{article.source?.name}</span>
              <span aria-hidden>·</span>
              <PublishedAt at={article.publishedAt} className="shrink-0" />
            </p>
          </div>
          <Button
            variant="danger"
            icon={<Trash2 className="size-4" />}
            onClick={() => setConfirming(true)}
          >
            {t("staff.removeArticle")}
          </Button>
        </div>
      )}
      <ConfirmDialog
        open={confirming}
        onOpenChange={setConfirming}
        title={t("staff.removeArticleTitle")}
        description={t("staff.removeArticleBody")}
        confirmLabel={t("staff.removeArticle")}
        danger
        loading={remove.isPending}
        onConfirm={() => article && remove.mutate(article.id)}
      />
    </Pane>
  );
}
