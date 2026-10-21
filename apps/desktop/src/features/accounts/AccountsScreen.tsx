import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { Account } from "../../bindings";
import {
  useAccounts,
  useAddOfflineAccount,
  useRemoveAccount,
  useSetDefaultAccount,
  useSettings,
  useYggdrasilLogin,
  useYggdrasilRegister,
  useYggdrasilServers,
} from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Card, CardList, Section } from "../../components/ui/Card";
import { Icon } from "../../components/ui/Icon";
import { ConfirmDialog } from "../../components/ui/Modal";
import { Field, Segmented, TextInput } from "../../components/ui/form";
import { Badge, EmptyState, ErrorMessage, Skeleton } from "../../components/ui/feedback";
import { isNotImplemented } from "../../lib/errors";
import { NICK_MAX, type RegisterDraft, hasErrors, validateNick, validateRegister } from "../../lib/validation";
import { SkinFlat } from "../skins/SkinPreview";

function OfflineAddForm() {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [touched, setTouched] = useState(false);
  const add = useAddOfflineAccount();
  const error = validateNick(name);

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        setTouched(true);
        if (error) return;
        add.mutate(name, {
          onSuccess: () => {
            setName("");
            setTouched(false);
          },
        });
      }}
    >
      <div className="flex items-start gap-2">
        <Field className="flex-1" label={t("accounts.nick")} error={touched && name !== "" ? error : null} hint={t("accounts.nickHint")}>
          {(id, d, inv) => (
            <TextInput
              id={id}
              aria-describedby={d}
              invalid={inv}
              value={name}
              maxLength={NICK_MAX + 8}
              placeholder={t("accounts.nickPlaceholder")}
              onChange={(e) => {
                setName(e.target.value);
                setTouched(true);
                add.reset();
              }}
            />
          )}
        </Field>
        <Button type="submit" variant="primary" className="mt-7" disabled={!!error} loading={add.isPending} icon={<Icon name="plus" />}>
          {t("accounts.add")}
        </Button>
      </div>
      <ErrorMessage error={add.error} />
    </form>
  );
}

type YggMode = "login" | "register";

/** Account server: `Settings.skinServer` wins over the built-in official server. */
function useAccountServer() {
  const servers = useYggdrasilServers();
  const settings = useSettings();
  const custom = settings.data?.skinServer ?? null;
  const official = servers.data?.find((s) => s.official) ?? servers.data?.[0];
  const server = custom ? { name: custom, apiRoot: custom } : official ? { name: official.name, apiRoot: official.apiRoot } : null;
  return { server, servers };
}

function SoonNotice() {
  const { t } = useTranslation();
  return (
    <div role="status" className="flex items-center gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-sm text-warning">
      <Badge tone="warning">{t("common.soon")}</Badge>
      {t("accounts.yggdrasil.soon")}
    </div>
  );
}

function YggdrasilSection() {
  const { t } = useTranslation();
  const [mode, setMode] = useState<YggMode>("login");
  return (
    <div className="flex flex-col gap-4">
      <Segmented<YggMode>
        ariaLabel={t("accounts.yggdrasil.mode")}
        value={mode}
        onChange={setMode}
        options={[
          { value: "login", label: t("accounts.yggdrasil.login") },
          { value: "register", label: t("accounts.yggdrasil.register") },
        ]}
      />
      {mode === "login" ? <YggdrasilForm /> : <RegisterForm />}
    </div>
  );
}

function RegisterForm() {
  const { t } = useTranslation();
  const { server, servers } = useAccountServer();
  const register = useYggdrasilRegister();
  const [draft, setDraft] = useState<RegisterDraft>({ invite: "", username: "", password: "", confirm: "" });
  const [touched, setTouched] = useState(false);
  const errors = validateRegister(draft);
  const unavailable = isNotImplemented(servers.error) || isNotImplemented(register.error);
  const set = (patch: Partial<RegisterDraft>) => {
    setDraft((d) => ({ ...d, ...patch }));
    register.reset();
  };
  const err = (k: keyof RegisterDraft) => (touched ? (errors[k] ?? null) : null);

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        setTouched(true);
        if (hasErrors(errors) || !server) return;
        register.mutate(
          { apiRoot: server.apiRoot, invite: draft.invite.trim(), username: draft.username, password: draft.password },
          {
            onSettled: () => setDraft((d) => ({ ...d, password: "", confirm: "" })),
            onSuccess: () => {
              setDraft({ invite: "", username: "", password: "", confirm: "" });
              setTouched(false);
            },
          },
        );
      }}
    >
      {unavailable ? <SoonNotice /> : null}
      <Field label={t("accounts.yggdrasil.invite")} error={err("invite")} hint={t("accounts.yggdrasil.inviteHint")}>
        {(id, d, inv) => (
          <TextInput id={id} aria-describedby={d} invalid={inv} value={draft.invite} onChange={(e) => set({ invite: e.target.value })} />
        )}
      </Field>
      <Field label={t("accounts.nick")} error={err("username")} hint={t("accounts.nickHint")}>
        {(id, d, inv) => (
          <TextInput id={id} aria-describedby={d} invalid={inv} value={draft.username} autoComplete="username" onChange={(e) => set({ username: e.target.value })} />
        )}
      </Field>
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label={t("accounts.yggdrasil.password")} error={err("password")}>
          {(id, d, inv) => (
            <TextInput id={id} type="password" aria-describedby={d} invalid={inv} autoComplete="new-password" value={draft.password} onChange={(e) => set({ password: e.target.value })} />
          )}
        </Field>
        <Field label={t("accounts.yggdrasil.confirm")} error={err("confirm")}>
          {(id, d, inv) => (
            <TextInput id={id} type="password" aria-describedby={d} invalid={inv} autoComplete="new-password" value={draft.confirm} onChange={(e) => set({ confirm: e.target.value })} />
          )}
        </Field>
      </div>
      <div className="flex items-center justify-between gap-3">
        <p className="truncate text-xs text-muted">{server ? t("accounts.yggdrasil.server", { name: server.name }) : t("accounts.yggdrasil.noServer")}</p>
        <Button type="submit" variant="primary" disabled={!server || unavailable} loading={register.isPending}>
          {t("accounts.yggdrasil.register")}
        </Button>
      </div>
      {!isNotImplemented(register.error) ? <ErrorMessage error={register.error} /> : null}
    </form>
  );
}

function YggdrasilForm() {
  const { t } = useTranslation();
  const { server, servers } = useAccountServer();
  const login = useYggdrasilLogin();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const unavailable = isNotImplemented(servers.error) || isNotImplemented(login.error);
  const canSubmit = !!server && username.trim() !== "" && password !== "" && !unavailable;

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (!canSubmit || !server) return;
        login.mutate(
          { apiRoot: server.apiRoot, username: username.trim(), password },
          {
            // The password never stays in UI state longer than needed.
            onSettled: () => setPassword(""),
            onSuccess: () => setUsername(""),
          },
        );
      }}
    >
      {unavailable ? <SoonNotice /> : null}
      <div className="grid gap-3 sm:grid-cols-2">
        <Field label={t("accounts.yggdrasil.username")}>
          {(id) => (
            <TextInput
              id={id}
              value={username}
              autoComplete="username"
              disabled={unavailable}
              onChange={(e) => setUsername(e.target.value)}
            />
          )}
        </Field>
        <Field label={t("accounts.yggdrasil.password")}>
          {(id) => (
            <TextInput
              id={id}
              type="password"
              value={password}
              autoComplete="current-password"
              disabled={unavailable}
              onChange={(e) => setPassword(e.target.value)}
            />
          )}
        </Field>
      </div>
      <div className="flex items-center justify-between gap-3">
        <p className="text-xs text-muted">
          {server ? t("accounts.yggdrasil.server", { name: server.name }) : t("accounts.yggdrasil.noServer")}
        </p>
        <Button type="submit" variant="primary" disabled={!canSubmit} loading={login.isPending}>
          {t("accounts.yggdrasil.login")}
        </Button>
      </div>
      {!isNotImplemented(login.error) ? <ErrorMessage error={login.error} /> : null}
      {servers.error && !isNotImplemented(servers.error) ? <ErrorMessage error={servers.error} /> : null}
    </form>
  );
}

function AccountRow({ account, onRemove }: { account: Account; onRemove: () => void }) {
  const { t } = useTranslation();
  const setDefault = useSetDefaultAccount();
  return (
    <Card className="flex flex-col gap-2">
      <div className="flex items-center gap-3">
        {account.skinUrl ? (
          <SkinFlat url={account.skinUrl} model="classic" variant="head" className="size-10 shrink-0 rounded-lg" />
        ) : (
          <span className="grid size-10 shrink-0 place-items-center rounded-lg bg-surface-2 font-semibold uppercase">
            {account.name.slice(0, 1)}
          </span>
        )}
        <div className="min-w-0 flex-1">
          <p className="flex items-center gap-2 truncate font-medium">
            {account.name}
            {account.isDefault ? <Badge tone="accent">{t("accounts.default")}</Badge> : null}
            {!account.valid ? <Badge tone="danger">{t("accounts.invalid")}</Badge> : null}
          </p>
          <p className="truncate text-xs text-muted">
            {t(`accounts.kind.${account.kind}`)}
            {account.server ? ` · ${account.server}` : ""}
          </p>
        </div>
        {!account.isDefault ? (
          <Button size="sm" variant="ghost" icon={<Icon name="star" />} loading={setDefault.isPending} onClick={() => setDefault.mutate(account.id)}>
            {t("accounts.makeDefault")}
          </Button>
        ) : null}
        <Button size="sm" variant="ghost" className="text-danger" icon={<Icon name="trash" />} onClick={onRemove} aria-label={t("common.delete")} />
      </div>
      <ErrorMessage error={setDefault.error} />
    </Card>
  );
}

export function AccountsScreen() {
  const { t } = useTranslation();
  const accounts = useAccounts();
  const remove = useRemoveAccount();
  const [removing, setRemoving] = useState<Account | null>(null);

  return (
    <div className="flex flex-col gap-6">
      <Section title={t("accounts.list")}>
        {accounts.isPending ? (
          <Skeleton className="h-16" />
        ) : accounts.error ? (
          <ErrorMessage error={accounts.error} />
        ) : accounts.data.length === 0 ? (
          <EmptyState title={t("accounts.empty")} />
        ) : (
          <CardList className="flex flex-col gap-2">
            {accounts.data.map((a) => (
              <AccountRow
                key={a.id}
                account={a}
                onRemove={() => {
                  remove.reset();
                  setRemoving(a);
                }}
              />
            ))}
          </CardList>
        )}
      </Section>

      <Section title={t("accounts.offlineTitle")}>
        <OfflineAddForm />
      </Section>

      <Section title={t("accounts.yggdrasil.title")}>
        <YggdrasilSection />
      </Section>

      <ConfirmDialog
        open={removing !== null}
        danger
        title={t("accounts.removeTitle")}
        message={t("accounts.removeMessage", { name: removing?.name ?? "" })}
        confirmLabel={t("common.delete")}
        loading={remove.isPending}
        error={remove.error ? <ErrorMessage error={remove.error} /> : null}
        onCancel={() => setRemoving(null)}
        onConfirm={() => {
          if (removing) remove.mutate(removing.id, { onSuccess: () => setRemoving(null) });
        }}
      />
    </div>
  );
}
