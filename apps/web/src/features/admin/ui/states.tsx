import { RotateCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "#/components/ui/button";
import { Skeleton } from "#/components/ui/page";
import { errorMessage } from "#/lib/errors";

/** Shown in place of content that failed to load. */
export function ErrorState({ error, onRetry }: { error: unknown; onRetry?(): void }) {
  const { t } = useTranslation("common");
  return (
    <div role="alert" className="flex flex-col items-center gap-3 px-6 py-12 text-center">
      <div className="text-5xl">🌀</div>
      <p className="max-w-sm font-bold">{errorMessage(error)}</p>
      {onRetry && (
        <Button size="sm" variant="secondary" icon={<RotateCw className="size-4" />} onClick={onRetry}>
          {t("actions.retry")}
        </Button>
      )}
    </div>
  );
}

/** Placeholder rows while a table loads. */
export function RowsSkeleton({ rows = 6 }: { rows?: number }) {
  return (
    <div className="flex flex-col gap-2" aria-busy>
      {Array.from({ length: rows }, (_, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: identical placeholders.
        <Skeleton key={index} className="h-12" />
      ))}
    </div>
  );
}
