import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Language, MemoryCfg, Settings } from "../../bindings";
import { useSaveSettings, useSettings, useSystemInfo } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { Section } from "../../components/ui/Card";
import { Field, Segmented, Switch, TextInput } from "../../components/ui/form";
import { ErrorMessage, ListSkeleton } from "../../components/ui/feedback";
import {
  type ProxyDraft,
  type ProxyMode,
  draftToProxy,
  hasErrors,
  pathOrNull,
  proxyToDraft,
  validateMemory,
  validateOptionalPath,
  validateOptionalUrl,
  validateProxy,
} from "../../lib/validation";
import { useDraftValue } from "../../hooks/useDraftValue";
import { ACCENT_PRESETS, isHexColor } from "../../lib/color";
import { BACKGROUND_IDS, type BackgroundId, resolveBackground } from "../../lib/backdrop";
import { BackgroundPreview } from "../appearance/backgrounds";
import { MemoryEditor, useFormatMb } from "../common/MemoryEditor";
import { AboutSection } from "../updates/AboutSection";
import { VoiceSettingsSection } from "../voice/VoiceSettingsSection";

type ThemeChoice = "dark" | "light" | "system";

/**
 * Accent swatches + custom colour. The native colour input fires on every drag
 * step, so custom values are committed after a short pause (no IPC per pixel).
 */
export function AccentPicker({ value, onChange }: { value: string | null; onChange: (v: string | null) => void }) {
  const { t } = useTranslation();
  const [draft, setDraft] = useDraftValue<string | null>(value, onChange);
  const current = draft?.toLowerCase() ?? null;
  const custom = current !== null && !(ACCENT_PRESETS as readonly string[]).includes(current);
  const swatch = (selected: boolean) =>
    `grid size-8 cursor-pointer place-items-center rounded-full border-2 transition-transform hover:scale-110 ${
      selected ? "border-text" : "border-transparent"
    }`;
  return (
    <div className="flex flex-col gap-1.5">
      <span id="accent-label" className="text-sm font-medium">
        {t("settings.accent")}
      </span>
      <div role="radiogroup" aria-labelledby="accent-label" className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          role="radio"
          aria-checked={current === null}
          aria-label={t("settings.accentDefault")}
          title={t("settings.accentDefault")}
          onClick={() => setDraft(null)}
          className={`${swatch(current === null)} bg-surface-2 text-muted`}
        >
          <Icon name="refresh" className="size-4" />
        </button>
        {ACCENT_PRESETS.map((c) => (
          <button
            key={c}
            type="button"
            role="radio"
            aria-checked={current === c}
            aria-label={t("settings.accentSwatch", { color: c })}
            title={c}
            onClick={() => setDraft(c)}
            className={swatch(current === c)}
          >
            <span className="size-6 rounded-full" style={{ background: c }} aria-hidden />
          </button>
        ))}
        <label className={`${swatch(custom)} relative overflow-hidden`} title={t("settings.accentCustom")}>
          <span className="sr-only">{t("settings.accentCustom")}</span>
          <span
            className="size-6 rounded-full"
            style={{ background: custom && current ? current : "conic-gradient(red, yellow, lime, cyan, blue, magenta, red)" }}
            aria-hidden
          />
          <input
            type="color"
            className="absolute inset-0 cursor-pointer opacity-0"
            value={current ?? "#4ade80"}
            onChange={(e) => {
              if (isHexColor(e.target.value)) setDraft(e.target.value.toLowerCase());
            }}
          />
        </label>
      </div>
      <p className="text-xs text-muted">{t("settings.accentHint")}</p>
    </div>
  );
}

/** Settings save delay of the background picker: rapid clicks cost one save and one scene swap. */
export const BACKGROUND_SAVE_MS = 300;

/**
 * Built-in background previews (static, memoised) plus "none". The selection
 * updates at once; the save (and the window background) follow after a pause.
 */
export function BackgroundPicker({ value: saved, onChange: commit }: { value: BackgroundId | null; onChange: (v: BackgroundId | null) => void }) {
  const { t } = useTranslation();
  const [value, onChange] = useDraftValue<BackgroundId | null>(saved, commit, BACKGROUND_SAVE_MS);
  const tile = (selected: boolean) =>
    `relative aspect-video w-full cursor-pointer overflow-hidden rounded-lg border-2 text-left transition-colors ${
      selected ? "border-accent" : "border-border hover:border-muted"
    }`;
  return (
    <div className="flex flex-col gap-1.5">
      <span id="background-label" className="text-sm font-medium">
        {t("settings.background")}
      </span>
      <div role="radiogroup" aria-labelledby="background-label" className="grid grid-cols-3 gap-2 sm:grid-cols-5">
        <button type="button" role="radio" aria-checked={value === null} onClick={() => onChange(null)} className={`${tile(value === null)} bg-surface-2`}>
          <span className="absolute inset-0 grid place-items-center text-xs text-muted">{t("settings.backgroundNone")}</span>
        </button>
        {BACKGROUND_IDS.map((id) => (
          <button
            key={id}
            type="button"
            role="radio"
            aria-checked={value === id}
            aria-label={t(`settings.backgrounds.${id}`)}
            title={t(`settings.backgrounds.${id}`)}
            onClick={() => onChange(id)}
            className={tile(value === id)}
          >
            <BackgroundPreview id={id} />
            <span className="absolute inset-x-0 bottom-0 truncate bg-surface px-1.5 py-0.5 text-[0.7rem] text-text">
              {t(`settings.backgrounds.${id}`)}
            </span>
          </button>
        ))}
      </div>
      <p className="text-xs text-muted">{t("settings.backgroundHint")}</p>
    </div>
  );
}

/** Theme, language and performance flags: saved and applied immediately. */
function AppearanceSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation();
  const save = useSaveSettings();
  const set = (patch: Partial<Settings>) => save.mutate({ ...settings, ...patch });
  const theme: ThemeChoice = typeof settings.theme === "string" ? settings.theme : "dark";

  return (
    <Section title={t("settings.appearance")}>
      <div className="flex flex-col gap-1.5">
        <span className="text-sm font-medium">{t("settings.theme")}</span>
        <Segmented<ThemeChoice>
          ariaLabel={t("settings.theme")}
          value={theme}
          onChange={(v) => set({ theme: v })}
          options={[
            { value: "dark", label: t("settings.themeDark") },
            { value: "light", label: t("settings.themeLight") },
            { value: "system", label: t("settings.themeSystem") },
          ]}
        />
      </div>
      <AccentPicker value={settings.accentColor} onChange={(accentColor) => set({ accentColor })} />
      <div className="flex flex-col gap-1.5">
        <span className="text-sm font-medium">{t("settings.language")}</span>
        <Segmented<Language>
          ariaLabel={t("settings.language")}
          value={settings.language}
          onChange={(v) => set({ language: v })}
          options={[
            { value: "ru", label: t("settings.langRu") },
            { value: "en", label: t("settings.langEn") },
          ]}
        />
      </div>
      <Switch
        checked={settings.translucent && !settings.lightMode}
        disabled={settings.lightMode}
        onChange={(translucent) => set({ translucent })}
        label={t("settings.translucent")}
        description={t(settings.lightMode ? "settings.translucentLightHint" : "settings.translucentHint")}
      />
      <BackgroundPicker value={resolveBackground(settings.background)} onChange={(background) => set({ background })} />
      <Switch
        checked={settings.lightMode}
        onChange={(lightMode) => set({ lightMode })}
        label={t("settings.lightMode")}
        description={t("settings.lightModeHint")}
      />
      <Switch
        checked={settings.reduceMotion}
        onChange={(reduceMotion) => set({ reduceMotion })}
        label={t("settings.reduceMotion")}
        description={t("settings.reduceMotionHint")}
      />
      <ErrorMessage error={save.error} />
    </Section>
  );
}

interface GameDraft {
  gameDir: string;
  javaPath: string;
  memory: MemoryCfg;
  proxy: ProxyDraft;
  skinServer: string;
}

const toDraft = (s: Settings): GameDraft => ({
  gameDir: s.gameDir ?? "",
  javaPath: s.javaPath ?? "",
  memory: { ...s.defaultMemory },
  proxy: proxyToDraft(s.proxy),
  skinServer: s.skinServer ?? "",
});

const sameDraft = (a: GameDraft, b: GameDraft) => JSON.stringify(a) === JSON.stringify(b);

function ProxyEditor({
  value,
  onChange,
  errors,
}: {
  value: ProxyDraft;
  onChange: (v: ProxyDraft) => void;
  errors: ReturnType<typeof validateProxy>;
}) {
  const { t } = useTranslation();
  const off = value.mode === "off";
  return (
    <div className="flex flex-col gap-4">
      <Segmented<ProxyMode>
        ariaLabel={t("settings.proxy.mode")}
        value={value.mode}
        onChange={(mode) => onChange({ ...value, mode })}
        options={[
          { value: "off", label: t("settings.proxy.off") },
          { value: "http", label: t("settings.proxy.http") },
          { value: "socks5", label: t("settings.proxy.socks5") },
        ]}
      />
      {!off ? (
        <div className="grid gap-4 sm:grid-cols-[1fr_8rem]">
          <Field label={t("settings.proxy.host")} error={errors.host}>
            {(id, d, inv) => (
              <TextInput
                id={id}
                aria-describedby={d}
                invalid={inv}
                value={value.host}
                placeholder={t("settings.proxy.hostPlaceholder")}
                onChange={(e) => onChange({ ...value, host: e.target.value })}
              />
            )}
          </Field>
          <Field label={t("settings.proxy.port")} error={errors.port}>
            {(id, d, inv) => (
              <TextInput
                id={id}
                aria-describedby={d}
                invalid={inv}
                inputMode="numeric"
                value={value.port}
                placeholder={t(`settings.proxy.portPlaceholder.${value.mode === "socks5" ? "socks5" : "http"}`)}
                onChange={(e) => onChange({ ...value, port: e.target.value })}
              />
            )}
          </Field>
          <Field label={t("settings.proxy.username")} error={errors.username} hint={t("common.optional")}>
            {(id, d, inv) => (
              <TextInput
                id={id}
                aria-describedby={d}
                invalid={inv}
                value={value.username}
                onChange={(e) => onChange({ ...value, username: e.target.value })}
              />
            )}
          </Field>
          <Field label={t("settings.proxy.password")} error={errors.password}>
            {(id, d, inv) => (
              <TextInput
                id={id}
                type="password"
                aria-describedby={d}
                invalid={inv}
                autoComplete="new-password"
                value={value.password}
                onChange={(e) => onChange({ ...value, password: e.target.value })}
              />
            )}
          </Field>
        </div>
      ) : null}
    </div>
  );
}

function GameSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation();
  const formatMb = useFormatMb();
  const system = useSystemInfo();
  const save = useSaveSettings();
  const [draft, setDraft] = useState<GameDraft>(() => toDraft(settings));
  const [base, setBase] = useState<GameDraft>(() => toDraft(settings));

  // Follow backend changes unless the user has unsaved edits.
  const baseRef = useRef(base);
  useEffect(() => {
    const incoming = toDraft(settings);
    const prevBase = baseRef.current;
    if (sameDraft(incoming, prevBase)) return;
    baseRef.current = incoming;
    setDraft((d) => (sameDraft(d, prevBase) ? incoming : d));
    setBase(incoming);
  }, [settings]);

  const totalRam = system.data?.totalRamMb ?? null;
  const memErrors = validateMemory(draft.memory.minMb, draft.memory.maxMb, totalRam);
  const pathErrors = {
    gameDir: validateOptionalPath(draft.gameDir),
    javaPath: validateOptionalPath(draft.javaPath),
  };
  const serverError = validateOptionalUrl(draft.skinServer);
  const proxyErrors = validateProxy(draft.proxy);
  const invalid = hasErrors(memErrors) || hasErrors(pathErrors) || hasErrors(proxyErrors) || serverError !== null;
  const dirty = !sameDraft(draft, base);

  const ramHint = system.data
    ? t("memory.systemHint", {
        total: formatMb(system.data.totalRamMb),
        free: formatMb(system.data.freeRamMb),
        suggested: formatMb(Math.min(8192, Math.max(2048, Math.floor(system.data.totalRamMb / 4 / 256) * 256))),
      })
    : null;

  const onSave = () => {
    if (invalid) return;
    save.mutate({
      ...settings,
      gameDir: pathOrNull(draft.gameDir),
      javaPath: pathOrNull(draft.javaPath),
      defaultMemory: draft.memory,
      proxy: draftToProxy(draft.proxy),
      skinServer: pathOrNull(draft.skinServer)?.replace(/\/+$/, "") ?? null,
    });
  };

  return (
    <>
      <Section title={t("settings.game")}>
        <Field label={t("settings.gameDir")} error={pathErrors.gameDir} hint={t("settings.gameDirHint")}>
          {(id, d, inv) => (
            <TextInput
              id={id}
              aria-describedby={d}
              invalid={inv}
              value={draft.gameDir}
              placeholder={t("settings.defaultPlaceholder")}
              onChange={(e) => setDraft({ ...draft, gameDir: e.target.value })}
            />
          )}
        </Field>
        <Field label={t("settings.javaPath")} error={pathErrors.javaPath} hint={t("settings.javaPathHint")}>
          {(id, d, inv) => (
            <TextInput
              id={id}
              aria-describedby={d}
              invalid={inv}
              value={draft.javaPath}
              placeholder={t("settings.javaAutoPlaceholder")}
              onChange={(e) => setDraft({ ...draft, javaPath: e.target.value })}
            />
          )}
        </Field>
      </Section>

      <Section title={t("settings.memory")}>
        <MemoryEditor
          value={draft.memory}
          onChange={(memory) => setDraft({ ...draft, memory })}
          totalRamMb={totalRam}
          errors={memErrors}
          hint={ramHint}
        />
      </Section>

      <Section title={t("settings.proxy.title")}>
        <p className="-mt-2 text-xs text-muted">{t("settings.proxy.hint")}</p>
        <ProxyEditor value={draft.proxy} onChange={(proxy) => setDraft({ ...draft, proxy })} errors={proxyErrors} />
      </Section>

      <Section title={t("settings.skinServer.title")}>
        <Field label={t("settings.skinServer.url")} error={serverError} hint={t("settings.skinServer.hint")}>
          {(id, d, inv) => (
            <TextInput
              id={id}
              aria-describedby={d}
              invalid={inv}
              value={draft.skinServer}
              placeholder={t("settings.skinServer.placeholder")}
              onChange={(e) => setDraft({ ...draft, skinServer: e.target.value })}
            />
          )}
        </Field>
      </Section>

      <div className="sticky bottom-0 -mx-4 flex flex-wrap items-center justify-end gap-2 border-t border-border bg-bg px-4 py-3 lg:-mx-6 lg:px-6">
        <ErrorMessage error={save.error} className="mr-auto" />
        {dirty && invalid ? <p className="mr-auto text-xs text-danger">{t("settings.fixErrors")}</p> : null}
        <Button variant="ghost" disabled={!dirty} onClick={() => setDraft(base)}>
          {t("common.reset")}
        </Button>
        <Button variant="primary" disabled={!dirty || invalid} loading={save.isPending} onClick={onSave}>
          {t("common.save")}
        </Button>
      </div>
    </>
  );
}

export function SettingsScreen() {
  const settings = useSettings();
  if (settings.isPending) return <ListSkeleton rows={3} rowClassName="h-48" className="flex flex-col gap-6" />;
  if (settings.error) return <ErrorMessage error={settings.error} onRetry={() => void settings.refetch()} />;
  return (
    <div className="flex flex-col gap-6">
      <AppearanceSection settings={settings.data} />
      <VoiceSettingsSection />
      {/* Before the game section: its sticky save bar must stay the last element. */}
      <AboutSection settings={settings.data} />
      <GameSection settings={settings.data} />
    </div>
  );
}
