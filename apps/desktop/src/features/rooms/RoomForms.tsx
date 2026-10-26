import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { InstanceId, RoomAuthMode, RoomKind } from "../../bindings";
import { useAccounts, useCreateRoom, useCreateVoiceRoom, useInstances, useJoinRoom } from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Section } from "../../components/ui/Card";
import { Icon } from "../../components/ui/Icon";
import { ErrorMessage } from "../../components/ui/feedback";
import { Field, Segmented, Select, Switch, TextInput } from "../../components/ui/form";
import { toAppError } from "../../lib/errors";
import {
  ROOM_PASSWORD_MAX,
  authModeFor,
  effectiveAuthMode,
  parseJoinInput,
  passwordOrNull,
  validateRoomPassword,
} from "../../lib/rooms";
import { useHostPackStore } from "../../stores/hostPackStore";
import { useNavStore } from "../../stores/navStore";

/** Instance picker shared by both forms; defaults to the Play screen selection. */
function useInstancePick() {
  const instances = useInstances();
  const playInstance = useNavStore((s) => s.playInstance);
  const [picked, setPicked] = useState<InstanceId | null>(null);
  const list = instances.data ?? [];
  const value =
    (picked && list.some((i) => i.id === picked) ? picked : null) ??
    (playInstance && list.some((i) => i.id === playInstance) ? playInstance : null) ??
    list[0]?.id ??
    null;
  return { instances, value, setPicked };
}

function InstanceField({ pick }: { pick: ReturnType<typeof useInstancePick> }) {
  const { t } = useTranslation();
  const list = pick.instances.data ?? [];
  return (
    <Field label={t("rooms.instance")} hint={t("rooms.instanceHint")} error={!pick.instances.isPending && list.length === 0 ? "rooms.noInstances" : null}>
      {(id, d, inv) => (
        <Select
          id={id}
          aria-describedby={d}
          invalid={inv}
          value={pick.value ?? ""}
          disabled={list.length === 0}
          onChange={(e) => pick.setPicked(e.target.value)}
        >
          {list.map((i) => (
            <option key={i.id} value={i.id}>
              {i.name} · {i.mc}
            </option>
          ))}
        </Select>
      )}
    </Field>
  );
}

function PasswordField({ value, onChange, error }: { value: string; onChange: (v: string) => void; error: string | null }) {
  const { t } = useTranslation();
  return (
    <Field label={`${t("rooms.password")} · ${t("common.optional")}`} error={error} hint={t("rooms.passwordHint")}>
      {(id, d, inv) => (
        <TextInput
          id={id}
          type="password"
          aria-describedby={d}
          invalid={inv}
          autoComplete="new-password"
          maxLength={ROOM_PASSWORD_MAX}
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
      )}
    </Field>
  );
}

export function CreateRoomForm() {
  const { t } = useTranslation();
  const pick = useInstancePick();
  const create = useCreateRoom();
  const selectPlayInstance = useNavStore((s) => s.selectPlayInstance);
  const accounts = useAccounts();
  const defaultAccount = accounts.data?.find((a) => a.isDefault) ?? accounts.data?.[0] ?? null;
  const choice = authModeFor(defaultAccount);
  const [picked, setAuthMode] = useState<RoomAuthMode | null>(null);
  const authMode = effectiveAuthMode(choice, picked);
  const [password, setPassword] = useState("");
  const pwError = validateRoomPassword(password);

  return (
    <Section title={t("rooms.create.title")}>
      <p className="text-sm text-muted">{t("rooms.create.intro")}</p>
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (!pick.value || pwError) return;
          const instance = pick.value;
          create.mutate(
            { instance, authMode, password: passwordOrNull(password) },
            {
              onSuccess: () => {
                selectPlayInstance(instance);
                setPassword("");
              },
            },
          );
        }}
      >
        <InstanceField pick={pick} />
        <div className="flex flex-col gap-1.5">
          <span className="text-sm font-medium">{t("rooms.authMode.label")}</span>
          <Segmented<RoomAuthMode>
            ariaLabel={t("rooms.authMode.label")}
            value={authMode}
            onChange={setAuthMode}
            options={[
              { value: "yggdrasil", label: t("rooms.authMode.yggdrasil"), disabled: choice.disabled === "yggdrasil" },
              { value: "open", label: t("rooms.authMode.open"), disabled: choice.disabled === "open" },
            ]}
          />
          <p className="text-xs text-muted">{t(`rooms.authModeHint.${authMode}`)}</p>
          {choice.disabled ? (
            <p className="text-xs text-muted" data-testid="auth-mode-disabled">
              {t(`rooms.authModeDisabled.${choice.disabled}`)}
            </p>
          ) : null}
        </div>
        <PasswordField
          value={password}
          error={pwError}
          onChange={(v) => {
            setPassword(v);
            create.reset();
          }}
        />
        <div className="flex justify-end">
          <Button type="submit" variant="primary" icon={<Icon name="users" />} disabled={!pick.value || !!pwError} loading={create.isPending}>
            {t("rooms.create.submit")}
          </Button>
        </div>
        <ErrorMessage error={create.error} />
      </form>
    </Section>
  );
}

/** Voice-only room: chat and voice for players on a regular server, no instance. */
export function CreateVoiceRoomForm() {
  const { t } = useTranslation();
  const create = useCreateVoiceRoom();
  const [password, setPassword] = useState("");
  const pwError = validateRoomPassword(password);

  return (
    <Section title={t("rooms.voiceCreate.title")}>
      <p className="text-sm text-muted">{t("rooms.voiceCreate.intro")}</p>
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (pwError) return;
          create.mutate(passwordOrNull(password), { onSuccess: () => setPassword("") });
        }}
      >
        <PasswordField
          value={password}
          error={pwError}
          onChange={(v) => {
            setPassword(v);
            create.reset();
          }}
        />
        <div className="flex justify-end">
          <Button type="submit" variant="primary" icon={<Icon name="headphones" />} disabled={!!pwError} loading={create.isPending}>
            {t("rooms.voiceCreate.submit")}
          </Button>
        </div>
        <ErrorMessage error={create.error} />
      </form>
    </Section>
  );
}

export function JoinRoomForm() {
  const { t } = useTranslation();
  const pick = useInstancePick();
  const join = useJoinRoom();
  const selectPlayInstance = useNavStore((s) => s.selectPlayInstance);
  const [kind, setKind] = useState<RoomKind>("game");
  const voice = kind === "voice";
  // Game rooms: the instance is matched (or downloaded) after joining; picking one is optional.
  const [manual, setManual] = useState(false);
  const pickManually = !voice && manual;
  const [code, setCode] = useState("");
  const [password, setPassword] = useState("");
  const [touched, setTouched] = useState(false);
  // Deep link `join/<code>`: prefill once (also when the form is already shown).
  const pendingJoin = useNavStore((s) => s.pendingJoin);
  useEffect(() => {
    if (pendingJoin === null) return;
    const next = useNavStore.getState().takeJoin();
    if (next) {
      setCode(next);
      setTouched(true);
    }
  }, [pendingJoin]);
  const parsed = parseJoinInput(code);
  const codeError = parsed.ok ? null : parsed.error;
  const pwError = validateRoomPassword(password);
  const denied = join.error != null && toAppError(join.error).code === "permissionDenied";

  return (
    <Section title={t("rooms.join.title")}>
      <p className="text-sm text-muted">{t("rooms.join.intro")}</p>
      <form
        className="flex flex-col gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          setTouched(true);
          if (!parsed.ok || (pickManually && !pick.value) || pwError) return;
          const instance = pickManually ? pick.value : null;
          join.mutate(
            { code: parsed.code, instance, password: passwordOrNull(password) },
            {
              onSuccess: (room) => {
                if (instance) {
                  selectPlayInstance(instance);
                  const packs = useHostPackStore.getState();
                  packs.begin(room.roomId);
                  packs.attach(room.roomId, instance);
                }
                setCode("");
                setPassword("");
                setTouched(false);
              },
            },
          );
        }}
      >
        <Field label={t("rooms.join.code")} error={touched ? codeError : null} hint={t("rooms.join.codeHint")}>
          {(id, d, inv) => (
            <TextInput
              id={id}
              aria-describedby={d}
              invalid={inv}
              data-testid="join-code"
              className="font-mono"
              value={code}
              placeholder={t("rooms.join.codePlaceholder")}
              onChange={(e) => {
                setCode(e.target.value);
                join.reset();
              }}
            />
          )}
        </Field>
        <div className="flex flex-col gap-1.5">
          <span className="text-sm font-medium">{t("rooms.kind.label")}</span>
          <Segmented<RoomKind>
            ariaLabel={t("rooms.kind.label")}
            value={kind}
            onChange={(k) => {
              setKind(k);
              join.reset();
            }}
            options={[
              { value: "game", label: t("rooms.kind.game") },
              { value: "voice", label: t("rooms.kind.voice") },
            ]}
          />
          <p className="text-xs text-muted">{t(`rooms.kindHint.${kind}`)}</p>
        </div>
        {voice ? null : (
          <div className="flex flex-col gap-2">
            <Switch checked={manual} onChange={setManual} label={t("rooms.pack.manual")} description={t("rooms.pack.manualHint")} />
            {manual ? <InstanceField pick={pick} /> : null}
          </div>
        )}
        <PasswordField
          value={password}
          error={pwError}
          onChange={(v) => {
            setPassword(v);
            join.reset();
          }}
        />
        <div className="flex justify-end">
          <Button type="submit" variant="primary" icon={<Icon name="link" />} disabled={(pickManually && !pick.value) || !!pwError} loading={join.isPending}>
            {t("rooms.join.submit")}
          </Button>
        </div>
        {denied ? (
          <div role="alert" className="rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger">
            {t("rooms.join.denied")}
          </div>
        ) : (
          <ErrorMessage error={join.error} />
        )}
      </form>
    </Section>
  );
}
