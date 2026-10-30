import { type ReactNode, useLayoutEffect, useRef } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";

export interface ChatComposerProps {
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  onAttach: () => void;
  /** Sending/attaching blocked (offline or a pending action). */
  disabled: boolean;
  /** Small rules / file-limit hint under the field. */
  hint: ReactNode;
  /** Extra actions (share a pack) placed next to the send button. */
  tools?: ReactNode;
}

const MAX_HEIGHT = 160;

/** Input row pinned to the bottom: auto-growing field, attach, send. */
export function ChatComposer({
  value,
  onChange,
  onSend,
  onAttach,
  disabled,
  hint,
  tools,
}: ChatComposerProps) {
  const { t } = useTranslation();
  const field = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = field.current;
    if (!el) return;
    el.style.height = "auto";
    if (el.scrollHeight > 0)
      el.style.height = `${Math.min(el.scrollHeight, MAX_HEIGHT)}px`;
  }, [value]);
  const send = () => {
    if (!disabled && value.trim()) onSend();
  };
  return (
    <form
      className="shrink-0 border-t border-border bg-panel px-2 pt-2 pb-1"
      onSubmit={(e) => {
        e.preventDefault();
        send();
      }}
    >
      <div className="flex items-end gap-1.5">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="mb-0.5"
          aria-label={t("social.attach")}
          title={t("social.attach")}
          disabled={disabled}
          onClick={onAttach}
          icon={<Icon name="paperclip" />}
        />
        <textarea
          ref={field}
          rows={1}
          aria-label={t("social.message")}
          placeholder={t("social.message")}
          maxLength={4096}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              send();
            }
          }}
          className="max-h-40 min-h-10 min-w-0 flex-1 resize-none rounded-2xl border border-border bg-surface-2 px-3 py-2 text-sm text-text outline-none placeholder:text-muted focus:border-accent"
        />
        {tools}
        <Button
          type="submit"
          variant="primary"
          className="mb-0.5"
          aria-label={t("social.send")}
          title={t("social.send")}
          disabled={disabled || !value.trim()}
          icon={<Icon name="send" />}
        />
      </div>
      <p className="px-1 pt-1 text-[11px] text-muted">{hint}</p>
    </form>
  );
}
