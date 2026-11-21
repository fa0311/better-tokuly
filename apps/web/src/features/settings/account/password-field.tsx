import { Eye, EyeOff } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { TextField, type TextFieldProps } from "#/components/ui/field";

/** A password input with a show / hide toggle. */
export function PasswordField(props: Omit<TextFieldProps, "type" | "trailing">) {
  const { t } = useTranslation("settings");
  const [visible, setVisible] = useState(false);
  return (
    <TextField
      {...props}
      type={visible ? "text" : "password"}
      trailing={
        <button
          type="button"
          aria-label={visible ? t("account.password.hide") : t("account.password.show")}
          aria-pressed={visible}
          onClick={() => setVisible((value) => !value)}
          className="grid size-9 place-items-center rounded-full text-ink-soft hover:bg-surface-2"
        >
          {visible ? <EyeOff className="size-5" /> : <Eye className="size-5" />}
        </button>
      }
    />
  );
}
