import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useSendChat } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Section } from "../../components/ui/Card";
import { Icon } from "../../components/ui/Icon";
import { Badge, ErrorMessage } from "../../components/ui/feedback";
import { TextInput } from "../../components/ui/form";
import { CHAT_MAX, chatTime, validateChat } from "../../lib/rooms";
import { useRoomStore } from "../../stores/roomStore";

/**
 * Room chat. History is capped (`CHAT_HISTORY`) in the store, so the list
 * stays small enough to render without windowing.
 */
export function RoomChat() {
  const { t } = useTranslation();
  const messages = useRoomStore((s) => s.messages);
  const selfPeerId = useRoomStore((s) => s.selfPeerId);
  const markRead = useRoomStore((s) => s.markRead);
  const send = useSendChat();
  const [text, setText] = useState("");
  const listRef = useRef<HTMLOListElement>(null);
  const invalid = validateChat(text) !== null;

  useEffect(() => {
    markRead();
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages.length, markRead]);

  return (
    <Section title={t("rooms.chat.title")}>
      <ol
        ref={listRef}
        aria-label={t("rooms.chat.title")}
        aria-live="polite"
        className="selectable flex h-64 flex-col gap-2 overflow-y-auto rounded-lg bg-surface-2 p-3"
      >
        {messages.length === 0 ? <li className="m-auto text-sm text-muted">{t("rooms.chat.empty")}</li> : null}
        {messages.map((m) => {
          const own = selfPeerId !== null && m.fromPeer === selfPeerId;
          return (
            <li key={m.id} className={`flex max-w-[85%] flex-col gap-0.5 ${own ? "self-end items-end" : "items-start"}`}>
              <span className="flex items-center gap-1.5 text-xs text-muted">
                <span className="font-medium text-text">{own ? t("rooms.chat.you") : m.fromName}</span>
                <time dateTime={m.sentAt}>{chatTime(m.sentAt)}</time>
                {m.viaRelay ? <Badge tone="warning">{t("rooms.chat.viaRelay")}</Badge> : null}
              </span>
              <span className={`rounded-lg px-3 py-1.5 text-sm break-words whitespace-pre-wrap ${own ? "bg-accent/15" : "bg-surface"}`}>{m.text}</span>
            </li>
          );
        })}
      </ol>
      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          if (invalid || send.isPending) return;
          send.mutate(text.trim(), { onSuccess: () => setText("") });
        }}
      >
        <TextInput
          aria-label={t("rooms.chat.input")}
          placeholder={t("rooms.chat.placeholder")}
          maxLength={CHAT_MAX}
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            send.reset();
          }}
        />
        <Button type="submit" variant="primary" icon={<Icon name="send" />} disabled={invalid} loading={send.isPending} aria-label={t("rooms.chat.send")} />
      </form>
      <ErrorMessage error={send.error} />
    </Section>
  );
}
