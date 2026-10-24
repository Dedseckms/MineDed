import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Skin, SkinModel } from "../../bindings";
import {
  useAccounts,
  useAddSkin,
  useApplySkin,
  useCapes,
  useRemoveSkin,
  useSetCape,
  useSkins,
} from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Section } from "../../components/ui/Card";
import { Icon } from "../../components/ui/Icon";
import { ConfirmDialog } from "../../components/ui/Modal";
import { Field, Segmented, Select, TextInput } from "../../components/ui/form";
import { Badge, EmptyState, ErrorMessage, ListSkeleton } from "../../components/ui/feedback";
import { useMotionPrefs } from "../../hooks/useAppearance";
import { SKIN_EXTS, skinNameFromPath, validateFilePath, validateSkinName } from "../../lib/validation";
import { Skin3D, SkinFlat } from "./SkinPreview";
import { skinSrc } from "./skinSrc";

function AddSkinForm({ onAdded }: { onAdded: (s: Skin) => void }) {
  const { t } = useTranslation();
  const add = useAddSkin();
  const [path, setPath] = useState("");
  const [name, setName] = useState("");
  const [model, setModel] = useState<SkinModel>("classic");
  const [touched, setTouched] = useState(false);
  const pathError = validateFilePath(path, SKIN_EXTS);
  const effectiveName = name.trim() === "" ? skinNameFromPath(path) : name;
  const nameError = validateSkinName(effectiveName);

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        setTouched(true);
        if (pathError || nameError) return;
        add.mutate(
          { path: path.trim(), name: effectiveName.trim(), model },
          {
            onSuccess: (skin) => {
              setPath("");
              setName("");
              setTouched(false);
              onAdded(skin);
            },
          },
        );
      }}
    >
      <Field label={t("skins.path")} error={touched ? pathError : null} hint={t("skins.pathHint")}>
        {(id, d, inv) => (
          <TextInput
            id={id}
            aria-describedby={d}
            invalid={inv}
            value={path}
            placeholder={t("skins.pathPlaceholder")}
            onChange={(e) => {
              setPath(e.target.value);
              add.reset();
            }}
          />
        )}
      </Field>
      <div className="grid gap-3 sm:grid-cols-[1fr_auto]">
        <Field label={t("skins.name")} error={touched && path !== "" ? nameError : null}>
          {(id, d, inv) => (
            <TextInput
              id={id}
              aria-describedby={d}
              invalid={inv}
              value={name}
              placeholder={skinNameFromPath(path) || t("skins.namePlaceholder")}
              onChange={(e) => setName(e.target.value)}
            />
          )}
        </Field>
        <div className="flex flex-col gap-1.5">
          <span className="text-sm font-medium">{t("skins.model")}</span>
          <Segmented<SkinModel>
            ariaLabel={t("skins.model")}
            value={model}
            onChange={setModel}
            options={[
              { value: "classic", label: t("skins.models.classic") },
              { value: "slim", label: t("skins.models.slim") },
            ]}
          />
        </div>
      </div>
      <div className="flex justify-end">
        <Button type="submit" variant="primary" icon={<Icon name="plus" />} loading={add.isPending}>
          {t("skins.add")}
        </Button>
      </div>
      <ErrorMessage error={add.error} />
    </form>
  );
}

function SkinTile({ skin, selected, onSelect }: { skin: Skin; selected: boolean; onSelect: () => void }) {
  const { t } = useTranslation();
  const src = skinSrc(skin);
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className={`flex cursor-pointer flex-col items-center gap-2 rounded-xl border p-3 text-left transition-colors ${
        selected ? "border-accent bg-accent/10" : "border-border bg-surface hover:bg-surface-2"
      }`}
    >
      {src ? <SkinFlat url={src} model={skin.model} className="h-28" /> : <span className="h-28 w-14 rounded-md bg-surface-2" aria-hidden />}
      <span className="w-full truncate text-center text-sm font-medium">{skin.name}</span>
      <span className="flex gap-1">
        <Badge>{t(`skins.models.${skin.model}`)}</Badge>
        {skin.localOnly ? <Badge tone="info">{t("skins.localOnly")}</Badge> : null}
      </span>
    </button>
  );
}

function CapePicker({ account, onPreview }: { account: string; onPreview: (url: string | null) => void }) {
  const { t } = useTranslation();
  const capes = useCapes(account);
  const setCape = useSetCape();
  const active = capes.data?.find((c) => c.active) ?? null;

  useEffect(() => onPreview(active?.url ?? null), [active?.url, onPreview]);

  if (capes.isPending) return <ListSkeleton rows={1} rowClassName="h-10" />;
  if (capes.error) return <ErrorMessage error={capes.error} onRetry={() => void capes.refetch()} />;
  if (capes.data.length === 0) return <p className="text-sm text-muted">{t("skins.noCapes")}</p>;
  return (
    <div className="flex flex-col gap-2">
      <Field label={t("skins.cape")}>
        {(id) => (
          <Select
            id={id}
            value={active?.id ?? ""}
            disabled={setCape.isPending}
            onChange={(e) => setCape.mutate({ account, capeId: e.target.value === "" ? null : e.target.value })}
          >
            <option value="">{t("skins.noCape")}</option>
            {capes.data.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </Select>
        )}
      </Field>
      <ErrorMessage error={setCape.error} />
    </div>
  );
}

/** Skin library: grid of flat thumbnails + one interactive 3D preview. */
export function SkinsScreen() {
  const { t } = useTranslation();
  const { light, reduced } = useMotionPrefs();
  const skins = useSkins();
  const accounts = useAccounts();
  const apply = useApplySkin();
  const remove = useRemoveSkin();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [accountId, setAccountId] = useState<string | null>(null);
  const [capeUrl, setCapeUrl] = useState<string | null>(null);
  const [removing, setRemoving] = useState<Skin | null>(null);

  const list = skins.data ?? [];
  const selected = list.find((s) => s.id === selectedId) ?? list[0] ?? null;
  const account = accounts.data?.find((a) => a.id === accountId) ?? accounts.data?.find((a) => a.isDefault) ?? accounts.data?.[0] ?? null;
  const selectedSrc = selected ? skinSrc(selected) : null;
  const appliedHere = !!selected && !!account && account.skinUrl === selected.url;

  return (
    <div className="flex flex-col gap-6">
      <div className="grid gap-6 lg:grid-cols-[1fr_18rem]">
        <Section title={t("skins.library")}>
          {skins.isPending ? (
            <ListSkeleton rows={6} rowClassName="h-44" className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4" />
          ) : skins.error ? (
            <ErrorMessage error={skins.error} onRetry={() => void skins.refetch()} />
          ) : list.length === 0 ? (
            <EmptyState icon="shirt" title={t("skins.empty")} />
          ) : (
            <div role="radiogroup" aria-label={t("skins.library")} className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4">
              {list.map((s) => (
                <SkinTile key={s.id} skin={s} selected={s.id === selected?.id} onSelect={() => setSelectedId(s.id)} />
              ))}
            </div>
          )}
        </Section>

        <Section title={t("skins.preview")}>
          {selected ? (
            <div className="flex flex-col gap-4">
              <div className="flex h-72 items-center justify-center rounded-lg bg-surface-2">
                {!selectedSrc ? (
                  <Icon name="shirt" className="size-16 text-muted" />
                ) : light ? (
                  <SkinFlat url={selectedSrc} model={selected.model} className="h-64" />
                ) : (
                  <Skin3D url={selectedSrc} model={selected.model} capeUrl={capeUrl} animate={!reduced} />
                )}
              </div>
              <p className="truncate text-center font-medium">{selected.name}</p>
              {accounts.data && accounts.data.length > 0 && account ? (
                <>
                  <Field label={t("skins.account")}>
                    {(id) => (
                      <Select id={id} value={account.id} onChange={(e) => setAccountId(e.target.value)}>
                        {accounts.data.map((a) => (
                          <option key={a.id} value={a.id}>
                            {a.name}
                          </option>
                        ))}
                      </Select>
                    )}
                  </Field>
                  <Button
                    variant="primary"
                    icon={<Icon name={appliedHere ? "check" : "shirt"} />}
                    disabled={appliedHere}
                    loading={apply.isPending}
                    onClick={() => apply.mutate({ account: account.id, skin: selected.id })}
                  >
                    {appliedHere ? t("skins.applied") : t("skins.apply")}
                  </Button>
                  <ErrorMessage error={apply.error} />
                  <CapePicker key={account.id} account={account.id} onPreview={setCapeUrl} />
                </>
              ) : (
                <p className="text-sm text-muted">{t("skins.noAccounts")}</p>
              )}
              <Button
                variant="ghost"
                className="text-danger"
                icon={<Icon name="trash" />}
                onClick={() => {
                  remove.reset();
                  setRemoving(selected);
                }}
              >
                {t("common.delete")}
              </Button>
            </div>
          ) : (
            <p className="text-sm text-muted">{t("skins.selectHint")}</p>
          )}
        </Section>
      </div>

      <Section title={t("skins.addTitle")}>
        <AddSkinForm onAdded={(s) => setSelectedId(s.id)} />
      </Section>

      <ConfirmDialog
        open={removing !== null}
        danger
        title={t("skins.removeTitle")}
        message={t("skins.removeMessage", { name: removing?.name ?? "" })}
        confirmLabel={t("common.delete")}
        loading={remove.isPending}
        error={remove.error ? <ErrorMessage error={remove.error} /> : null}
        onCancel={() => setRemoving(null)}
        onConfirm={() => {
          if (removing)
            remove.mutate(removing.id, {
              onSuccess: () => {
                setRemoving(null);
                setSelectedId(null);
              },
            });
        }}
      />
    </div>
  );
}
