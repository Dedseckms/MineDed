import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Skin, SkinModel } from "../../bindings";
import { pickFile } from "../../api/dialog";
import {
  useAccounts,
  useAddCape,
  useAddSkin,
  useApplySkin,
  useCapes,
  useFetchRemoteSkin,
  useRemoveSkin,
  useSetCape,
  useSkins,
} from "../../api/queries";
import { Button } from "../../components/ui/Button";
import { Section } from "../../components/ui/Card";
import { Icon } from "../../components/ui/Icon";
import { ConfirmDialog, Modal } from "../../components/ui/Modal";
import { Field, Segmented, Select, TextInput } from "../../components/ui/form";
import { Badge, EmptyState, ErrorMessage, ListSkeleton } from "../../components/ui/feedback";
import { useMotionPrefs } from "../../hooks/useAppearance";
import { fileName } from "../../lib/path";
import { SKIN_NAME_MAX, skinNameFromPath, validateSkinName, validateSkinSource } from "../../lib/validation";
import { Skin3D, SkinFlat } from "./SkinPreview";
import { localSkinSrc, skinSrc } from "./skinSrc";

type AddSource = "computer" | "web";

/** A PNG ready to be added: picked from disk or downloaded by `skin_fetch_remote`. */
interface Candidate {
  path: string;
  name: string;
  model: SkinModel;
}

function AddSkinDialog({ open, onClose, onAdded }: { open: boolean; onClose: () => void; onAdded: (s: Skin) => void }) {
  const { t } = useTranslation();
  const { light, reduced } = useMotionPrefs();
  const add = useAddSkin();
  const fetchRemote = useFetchRemoteSkin();
  const [source, setSource] = useState<AddSource>("computer");
  const [web, setWeb] = useState("");
  const [webTouched, setWebTouched] = useState(false);
  const [cand, setCand] = useState<Candidate | null>(null);
  const [touched, setTouched] = useState(false);
  const [pickError, setPickError] = useState<unknown>(null);

  const webError = validateSkinSource(web);
  const nameError = cand ? validateSkinName(cand.name) : null;
  const src = cand ? localSkinSrc(cand.path) : null;

  const reset = () => {
    setCand(null);
    setTouched(false);
    setPickError(null);
    add.reset();
    fetchRemote.reset();
  };
  const close = () => {
    reset();
    setWeb("");
    setWebTouched(false);
    onClose();
  };

  const pick = async () => {
    reset();
    try {
      const path = await pickFile([{ name: t("dialog.png"), extensions: ["png"] }]);
      if (path) setCand({ path, name: skinNameFromPath(path), model: "classic" });
    } catch (e) {
      setPickError(e);
    }
  };

  const find = () => {
    setWebTouched(true);
    if (webError) return;
    reset();
    fetchRemote.mutate(web.trim(), {
      onSuccess: (r) => setCand({ path: r.localPath, name: r.suggestedName.slice(0, SKIN_NAME_MAX), model: r.model }),
    });
  };

  const submit = () => {
    setTouched(true);
    if (!cand || nameError) return;
    add.mutate(
      { path: cand.path, name: cand.name.trim(), model: cand.model },
      {
        onSuccess: (skin) => {
          onAdded(skin);
          close();
        },
      },
    );
  };

  return (
    <Modal
      open={open}
      onClose={close}
      wide
      title={t("skins.addTitle")}
      footer={
        <>
          <Button variant="ghost" onClick={close}>
            {t("common.cancel")}
          </Button>
          <Button variant="primary" icon={<Icon name="plus" />} disabled={!cand} loading={add.isPending} onClick={submit}>
            {t("skins.add")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-4">
        <Segmented<AddSource>
          ariaLabel={t("skins.source")}
          value={source}
          onChange={(s) => {
            setSource(s);
            reset();
          }}
          options={[
            { value: "computer", label: t("skins.fromComputer") },
            { value: "web", label: t("skins.fromWeb") },
          ]}
        />
        {source === "computer" ? (
          <div className="flex flex-wrap items-center gap-3">
            <Button icon={<Icon name="folder" />} onClick={() => void pick()}>
              {t("skins.pickFile")}
            </Button>
            <p className="min-w-0 flex-1 text-xs text-muted">{t("skins.fileHint")}</p>
          </div>
        ) : (
          <form
            className="flex items-start gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              find();
            }}
          >
            <Field className="flex-1" label={t("skins.webSource")} error={webTouched ? webError : null} hint={t("skins.webHint")}>
              {(id, d, inv) => (
                <TextInput
                  id={id}
                  aria-describedby={d}
                  invalid={inv}
                  value={web}
                  spellCheck={false}
                  autoComplete="off"
                  placeholder={t("skins.webPlaceholder")}
                  onChange={(e) => {
                    setWeb(e.target.value);
                    fetchRemote.reset();
                  }}
                />
              )}
            </Field>
            <Button type="submit" className="mt-7" icon={<Icon name="search" />} loading={fetchRemote.isPending}>
              {t("skins.find")}
            </Button>
          </form>
        )}
        <ErrorMessage error={fetchRemote.error ?? pickError} />

        {cand ? (
          <div className="grid min-w-0 gap-4 sm:grid-cols-[12rem_minmax(0,1fr)]">
            <div className="flex h-60 items-center justify-center rounded-lg bg-surface-2" aria-label={t("skins.candidate")}>
              {!src ? (
                <Icon name="shirt" className="size-16 text-muted" />
              ) : light ? (
                <SkinFlat url={src} model={cand.model} className="h-52" />
              ) : (
                <Skin3D url={src} model={cand.model} capeUrl={null} animate={!reduced} />
              )}
            </div>
            <div className="flex min-w-0 flex-col gap-3">
              <p
                className="min-w-0 truncate whitespace-nowrap font-mono text-xs text-muted"
                title={cand.path.startsWith("data:") ? undefined : cand.path}
                data-testid="skin-file"
              >
                {fileName(cand.path) || cand.name}
              </p>
              <Field label={t("skins.name")} error={touched ? nameError : null}>
                {(id, d, inv) => (
                  <TextInput
                    id={id}
                    aria-describedby={d}
                    invalid={inv}
                    value={cand.name}
                    maxLength={SKIN_NAME_MAX}
                    placeholder={t("skins.namePlaceholder")}
                    onChange={(e) => setCand({ ...cand, name: e.target.value })}
                  />
                )}
              </Field>
              <div className="flex flex-col gap-1.5">
                <span className="text-sm font-medium">{t("skins.model")}</span>
                <Segmented<SkinModel>
                  ariaLabel={t("skins.model")}
                  value={cand.model}
                  onChange={(model) => setCand({ ...cand, model })}
                  options={[
                    { value: "classic", label: t("skins.models.classic") },
                    { value: "slim", label: t("skins.models.slim") },
                  ]}
                />
              </div>
            </div>
          </div>
        ) : null}
        <ErrorMessage error={add.error} />
      </div>
    </Modal>
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
  const add = useAddCape();
  const setCape = useSetCape();
  const [selectedId, setSelectedId] = useState<string | null | undefined>(undefined);
  const [web, setWeb] = useState("");
  const [pickError, setPickError] = useState<unknown>(null);
  const active = capes.data?.find((c) => c.active) ?? null;
  const id = selectedId === undefined ? active?.id ?? null : selectedId;
  const selected = capes.data?.find((c) => c.id === id) ?? (add.data?.id === id ? add.data : null);
  const preview = selected ? localSkinSrc(selected.url) : null;
  const pending = add.isPending || setCape.isPending;

  useEffect(() => onPreview(preview), [preview, onPreview]);

  const importCape = (source: string) => {
    setPickError(null);
    add.mutate(source, { onSuccess: (cape) => setSelectedId(cape.id) });
  };
  const pick = async () => {
    try {
      const path = await pickFile([{ name: t("dialog.png"), extensions: ["png"] }]);
      if (path) importCape(path);
    } catch (e) { setPickError(e); }
  };

  return (
    <div className="flex flex-col gap-2">
      <p className="text-xs text-muted">{t("skins.capeHint")}</p>
      {capes.isPending ? <ListSkeleton rows={1} rowClassName="h-10" /> : null}
      <ErrorMessage error={capes.error} onRetry={() => void capes.refetch()} />
      <Field label={t("skins.cape")}>
        {(inputId) => (
          <Select id={inputId} value={id ?? ""} disabled={pending || capes.isPending}
            onChange={(e) => setSelectedId(e.target.value || null)}>
            <option value="">{t("skins.noCape")}</option>
            {(capes.data ?? []).map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}
          </Select>
        )}
      </Field>
      {preview ? <img src={preview} alt={t("skins.capePreview")} className="h-16 w-32 self-center object-contain [image-rendering:pixelated]" /> : null}
      <Button disabled={pending} loading={add.isPending} onClick={() => void pick()}>{t("skins.addCapeFile")}</Button>
      <form className="flex flex-col gap-2" onSubmit={(e) => { e.preventDefault(); importCape(web.trim()); }}>
        <Field label={t("skins.capeUrl")}>
          {(inputId) => <TextInput id={inputId} type="url" value={web} placeholder={t("skins.capeUrlPlaceholder")}
            onChange={(e) => setWeb(e.target.value)} />}
        </Field>
        <Button type="submit" disabled={pending || !/^https:\/\//i.test(web.trim())} loading={add.isPending}>{t("skins.addCapeUrl")}</Button>
      </form>
      <Button variant="primary" disabled={pending || id === (active?.id ?? null)} loading={setCape.isPending}
        onClick={() => setCape.mutate({ account, capeId: id })}>
        {id === null ? t("skins.removeCape") : t("skins.applyCape")}
      </Button>
      <ErrorMessage error={add.error ?? pickError ?? setCape.error} />
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
  const [adding, setAdding] = useState(false);

  const list = skins.data ?? [];
  const selected = list.find((s) => s.id === selectedId) ?? list[0] ?? null;
  const account = accounts.data?.find((a) => a.id === accountId) ?? accounts.data?.find((a) => a.isDefault) ?? accounts.data?.[0] ?? null;
  const selectedSrc = selected ? skinSrc(selected) : null;
  const appliedHere = !!selected && !!account && account.skinUrl === selected.url;

  return (
    <div className="flex flex-col gap-6">
      <div className="grid gap-6 lg:grid-cols-[1fr_18rem]">
        <Section
          title={t("skins.library")}
          actions={
            <Button size="sm" variant="primary" icon={<Icon name="plus" />} onClick={() => setAdding(true)}>
              {t("skins.addButton")}
            </Button>
          }
        >
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
                ) : light || adding ? (
                  // One WebGL context at a time: the add dialog has its own 3D preview.
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

      <AddSkinDialog open={adding} onClose={() => setAdding(false)} onAdded={(s) => setSelectedId(s.id)} />

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
