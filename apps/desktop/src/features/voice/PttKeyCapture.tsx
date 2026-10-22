import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { captureKey, keyLabel } from "../../lib/voice";

/** "Press a key to bind" control; stores `KeyboardEvent.code` (keyboard-types naming). */
export function PttKeyCapture({ value, onChange }: { value: string | null; onChange: (code: string | null) => void }) {
  const { t } = useTranslation();
  const [listening, setListening] = useState(false);

  useEffect(() => {
    if (!listening) return;
    const onKey = (e: KeyboardEvent) => {
      const r = captureKey(e.code);
      if (r.kind === "ignore") return;
      e.preventDefault();
      e.stopPropagation();
      setListening(false);
      if (r.kind === "set") onChange(r.code);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [listening, onChange]);

  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-sm font-medium">{t("voice.settings.pttKey")}</span>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant={listening ? "primary" : "secondary"}
          icon={<Icon name="keyboard" />}
          aria-pressed={listening}
          onClick={() => setListening((v) => !v)}
          data-testid="ptt-capture"
        >
          {listening ? t("voice.settings.pttListening") : value ? keyLabel(value) : t("voice.settings.pttNone")}
        </Button>
        {value && !listening ? (
          <Button variant="ghost" size="sm" onClick={() => onChange(null)}>
            {t("voice.settings.pttClear")}
          </Button>
        ) : null}
        {value ? <code className="text-xs text-muted">{value}</code> : null}
      </div>
      <p className="text-xs text-muted">{listening ? t("voice.settings.pttListeningHint") : t("voice.settings.pttHint")}</p>
    </div>
  );
}
