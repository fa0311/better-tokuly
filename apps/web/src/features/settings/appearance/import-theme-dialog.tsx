import { parseThemeFile, type ThemeColors } from "@tokuly/core/theme";
import { FileUp } from "lucide-react";
import { useRef, useState } from "react";
import { toast } from "sonner";
import { Button } from "#/components/ui/button";
import { Dialog } from "#/components/ui/dialog";
import { TextArea } from "#/components/ui/field";
import { useTranslation } from "#/lib/use-translation";

/** Takes a theme file in, pasted or chosen; its colours replace the editor's. */
export function ImportThemeDialog({
  open,
  onOpenChange,
  onImport,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  onImport(colors: ThemeColors): void;
}) {
  const { t } = useTranslation("themeEditor");
  const [text, setText] = useState("");
  const [error, setError] = useState<string>();
  const fileInput = useRef<HTMLInputElement>(null);

  const close = (next: boolean) => {
    onOpenChange(next);
    if (!next) {
      setText("");
      setError(undefined);
    }
  };

  const submit = (json: string) => {
    const result = parseThemeFile(json);
    if ("error" in result) {
      setError(t(`import.errors.${result.error}`));
      return;
    }
    onImport(result.colors);
    close(false);
    toast.success(t("import.done"), {
      description: result.skipped ? t("import.skipped", { count: result.skipped }) : undefined,
    });
  };

  return (
    <Dialog
      open={open}
      onOpenChange={close}
      title={t("import.title")}
      description={t("import.description")}
      footer={
        <>
          <Button
            variant="secondary"
            icon={<FileUp className="size-4" />}
            onClick={() => fileInput.current?.click()}
          >
            {t("import.file")}
          </Button>
          <Button disabled={!text.trim()} onClick={() => submit(text)}>
            {t("import.submit")}
          </Button>
        </>
      }
    >
      <TextArea
        label={t("import.label")}
        value={text}
        error={error}
        spellCheck={false}
        className="font-mono"
        placeholder='{ "format": "tokuly-theme", … }'
        onChange={(event) => {
          setText(event.target.value);
          setError(undefined);
        }}
      />
      <input
        ref={fileInput}
        type="file"
        accept="application/json,.json"
        hidden
        onChange={async (event) => {
          const file = event.target.files?.[0];
          event.target.value = "";
          if (file) submit(await file.text());
        }}
      />
    </Dialog>
  );
}
