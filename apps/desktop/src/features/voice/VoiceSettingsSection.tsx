import { useCallback, useEffect } from "react";
import { useTranslation } from "react-i18next";

import type { VoiceMode, VoiceSettings } from "../../bindings";
import {
  useMicTest,
  useSaveVoiceSettings,
  useSystemInfo,
  useVoiceDevices,
  useVoiceSettings,
} from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Section } from "../../components/ui/Card";
import { Icon } from "../../components/ui/Icon";
import { LevelMeter, PercentSlider } from "../../components/ui/LevelMeter";
import { ErrorMessage, Skeleton } from "../../components/ui/feedback";
import { Field, Segmented, Select, Switch } from "../../components/ui/form";
import { useDraftValue } from "../../hooks/useDraftValue";
import { deviceValue, devicesOf, fromPercent, toPercent } from "../../lib/voice";
import { useVoiceStore } from "../../stores/voiceStore";
import { PttKeyCapture } from "./PttKeyCapture";

function WaylandHint() {
  const { t } = useTranslation();
  const system = useSystemInfo();
  if (!system.data?.wayland) return null;
  return (
    <p role="note" className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning">
      {t("voice.settings.waylandPtt")}
    </p>
  );
}

function MicTest() {
  const { t } = useTranslation();
  const test = useMicTest();
  const on = useVoiceStore((s) => s.micTest);
  const level = useVoiceStore((s) => s.selfLevel);
  const mutate = test.mutate;

  // Never leave the microphone loopback running after leaving the screen.
  useEffect(
    () => () => {
      if (useVoiceStore.getState().micTest) mutate(false);
    },
    [mutate],
  );

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-3">
        <Button
          variant={on ? "danger" : "secondary"}
          icon={<Icon name="mic" />}
          loading={test.isPending}
          aria-pressed={on}
          onClick={() => test.mutate(!on)}
        >
          {on ? t("voice.settings.micTestStop") : t("voice.settings.micTest")}
        </Button>
        <LevelMeter level={on ? level : 0} label={t("voice.settings.micLevel")} className="flex-1" />
      </div>
      <p className="text-xs text-muted">{t("voice.settings.micTestHint")}</p>
      <ErrorMessage error={test.error} />
    </div>
  );
}

function VoiceForm({ settings }: { settings: VoiceSettings }) {
  const { t } = useTranslation();
  const devices = useVoiceDevices();
  const save = useSaveVoiceSettings();
  const saveMutate = save.mutate;
  const set = useCallback((patch: Partial<VoiceSettings>) => saveMutate({ ...settings, ...patch }), [saveMutate, settings]);
  const inputs = devicesOf(devices.data, "input");
  const outputs = devicesOf(devices.data, "output");

  const [gain, setGain] = useDraftValue(toPercent(settings.inputGain), (p) => set({ inputGain: fromPercent(p) }));
  const [volume, setVolume] = useDraftValue(toPercent(settings.outputVolume), (p) => set({ outputVolume: fromPercent(p) }));
  const setPtt = useCallback((pttKey: string | null) => set({ pttKey }), [set]);

  return (
    <Section title={t("voice.settings.title")}>
      <div className="flex flex-col gap-1.5">
        <span className="text-sm font-medium">{t("voice.settings.mode")}</span>
        <Segmented<VoiceMode>
          ariaLabel={t("voice.settings.mode")}
          value={settings.mode}
          onChange={(mode) => set({ mode })}
          options={[
            { value: "pushToTalk", label: t("voice.mode.pushToTalk") },
            { value: "vad", label: t("voice.mode.vad") },
            { value: "off", label: t("voice.mode.off") },
          ]}
        />
        <p className="text-xs text-muted">{t(`voice.modeHint.${settings.mode}`)}</p>
      </div>

      {settings.mode === "pushToTalk" ? (
        <>
          <PttKeyCapture value={settings.pttKey} onChange={setPtt} />
          <WaylandHint />
        </>
      ) : null}

      <div className="grid gap-4 sm:grid-cols-2">
        <Field label={t("voice.settings.input")}>
          {(id) => (
            <Select
              id={id}
              value={deviceValue(inputs, settings.inputDevice)}
              disabled={devices.isPending}
              onChange={(e) => set({ inputDevice: e.target.value || null })}
            >
              <option value="">{t("voice.settings.systemDefault")}</option>
              {inputs.map((d) => (
                <option key={d.id} value={d.id}>
                  {d.name}
                </option>
              ))}
            </Select>
          )}
        </Field>
        <Field label={t("voice.settings.output")}>
          {(id) => (
            <Select
              id={id}
              value={deviceValue(outputs, settings.outputDevice)}
              disabled={devices.isPending}
              onChange={(e) => set({ outputDevice: e.target.value || null })}
            >
              <option value="">{t("voice.settings.systemDefault")}</option>
              {outputs.map((d) => (
                <option key={d.id} value={d.id}>
                  {d.name}
                </option>
              ))}
            </Select>
          )}
        </Field>
      </div>
      <ErrorMessage error={devices.error} />

      <PercentSlider label={t("voice.settings.inputGain")} value={gain} onChange={setGain} />
      <PercentSlider label={t("voice.settings.outputVolume")} value={volume} onChange={setVolume} />

      <Switch
        checked={settings.noiseSuppression}
        onChange={(noiseSuppression) => set({ noiseSuppression })}
        label={t("voice.settings.noiseSuppression")}
        description={t("voice.settings.noiseSuppressionHint")}
      />

      <MicTest />
      <ErrorMessage error={save.error} />
    </Section>
  );
}

/** Voice settings: mode, PTT key, devices, gains, noise suppression, mic test. */
export function VoiceSettingsSection() {
  const settings = useVoiceSettings();
  if (settings.isPending) return <Skeleton className="h-64" />;
  if (settings.error) return <ErrorMessage error={settings.error} />;
  return <VoiceForm settings={settings.data} />;
}
