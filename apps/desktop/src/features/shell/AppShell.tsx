import { AnimatePresence, MotionConfig, motion } from "motion/react";
import type { ComponentType } from "react";
import { useTranslation } from "react-i18next";

import { isMock } from "../../api/client";
import { BRAND_MARK } from "../../brand";
import { useAccounts, useAppVersion } from "../../api/queries";
import { Icon, type IconName } from "../../components/ui/Icon";
import { Toaster } from "../../components/ui/Toaster";
import { Badge } from "../../components/ui/feedback";
import { useApplyAppearance, useMotionPrefs, useMotionPreset } from "../../hooks/useAppearance";
import { INSTANT } from "../../lib/motion";
import { useBackendSync } from "../../hooks/useBackendSync";
import { SECTIONS, type Section, useNavStore } from "../../stores/navStore";
import { useRoomStore } from "../../stores/roomStore";
import { AccountsScreen } from "../accounts/AccountsScreen";
import { InstancesScreen } from "../instances/InstancesScreen";
import { PlayScreen } from "../play/PlayScreen";
import { SettingsScreen } from "../settings/SettingsScreen";
import { MultiplayerScreen } from "../rooms/MultiplayerScreen";
import { SkinsScreen } from "../skins/SkinsScreen";
import { VoiceHeaderToggle } from "../voice/VoicePanel";
import { VpnScreen } from "../vpn/VpnScreen";
import { UpdateBanner } from "../updates/UpdateBanner";
import { TaskCenter } from "./TaskCenter";

const ICONS: Record<Section, IconName> = {
  play: "play",
  instances: "layers",
  multiplayer: "users",
  vpn: "shield",
  skins: "shirt",
  accounts: "user",
  settings: "settings",
};

const SCREENS: Record<Section, ComponentType> = {
  play: PlayScreen,
  instances: InstancesScreen,
  multiplayer: MultiplayerScreen,
  vpn: VpnScreen,
  skins: SkinsScreen,
  accounts: AccountsScreen,
  settings: SettingsScreen,
};

function Sidebar() {
  const { t } = useTranslation();
  const section = useNavStore((s) => s.section);
  const go = useNavStore((s) => s.go);
  const version = useAppVersion();
  const unread = useRoomStore((s) => s.unread);
  const { reduced } = useMotionPrefs();
  // Below `lg` (e.g. 1024px windows) the sidebar collapses to icons; labels stay for screen readers.
  return (
    <nav
      aria-label={t("nav.label")}
      data-testid="sidebar"
      className="flex w-16 shrink-0 flex-col border-r border-border bg-surface px-2 py-4 lg:w-56 lg:px-3"
    >
      <div className="mb-6 flex items-center justify-center gap-2 lg:justify-start lg:px-3">
        <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-accent text-sm font-bold text-accent-fg" aria-hidden>
          {BRAND_MARK}
        </span>
        <span className="sr-only text-lg font-semibold tracking-tight lg:not-sr-only">{t("app.title")}</span>
      </div>
      <ul className="flex flex-col gap-1">
        {SECTIONS.map((s) => {
          const active = s === section;
          return (
            <li key={s}>
              <button
                type="button"
                aria-current={active ? "page" : undefined}
                onClick={() => go(s)}
                title={t(`nav.${s}`)}
                className={`relative flex h-10 w-full cursor-pointer items-center justify-center gap-3 rounded-lg px-3 text-sm font-medium transition-colors lg:justify-start ${
                  active ? "text-text" : "text-muted hover:bg-surface-2 hover:text-text"
                }`}
              >
                {active ? (
                  <motion.span
                    layoutId="nav-active"
                    className="absolute inset-0 rounded-lg bg-surface-2"
                    transition={reduced ? INSTANT : { type: "spring", stiffness: 500, damping: 40 }}
                  />
                ) : null}
                <span className="relative flex items-center gap-3">
                  <Icon name={ICONS[s]} />
                  <span className="sr-only lg:not-sr-only">{t(`nav.${s}`)}</span>
                  {s === "multiplayer" && unread > 0 && !active ? (
                    <span className="absolute -top-1.5 left-3 lg:static">
                      <Badge tone="accent">
                        <span aria-label={t("rooms.chat.unread", { n: unread })}>{unread > 99 ? "99+" : unread}</span>
                      </Badge>
                    </span>
                  ) : null}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
      <p className="mt-auto hidden px-3 text-xs text-muted lg:block">
        {t("app.version", { version: version.data ?? "…" })}
        {isMock ? ` · ${t("app.mock")}` : ""}
      </p>
    </nav>
  );
}

function CurrentAccount() {
  const { t } = useTranslation();
  const go = useNavStore((s) => s.go);
  const accounts = useAccounts();
  const current = accounts.data?.find((a) => a.isDefault) ?? accounts.data?.[0];
  return (
    <button
      type="button"
      onClick={() => go("accounts")}
      className="flex h-9 cursor-pointer items-center gap-2 rounded-lg px-2 text-sm hover:bg-surface-2"
      title={t("header.accountHint")}
    >
      <span className="grid size-7 place-items-center rounded-md bg-surface-2 text-xs font-semibold uppercase">
        {current ? current.name.slice(0, 1) : "?"}
      </span>
      {current ? (
        <span className="hidden flex-col items-start leading-tight sm:flex">
          <span className="font-medium">{current.name}</span>
          <span className="text-xs text-muted">{t(`accounts.kind.${current.kind}`)}</span>
        </span>
      ) : (
        <span className="hidden text-muted sm:inline">{accounts.isPending ? t("common.loading") : t("header.noAccount")}</span>
      )}
      {current && !current.valid ? <Badge tone="danger">{t("accounts.invalid")}</Badge> : null}
    </button>
  );
}

export function AppShell() {
  const { t } = useTranslation();
  useApplyAppearance();
  useBackendSync();
  const { reduced } = useMotionPrefs();
  const screen = useMotionPreset("screen");
  const section = useNavStore((s) => s.section);
  const Screen = SCREENS[section];

  return (
    <MotionConfig reducedMotion={reduced ? "always" : "user"} transition={reduced ? INSTANT : undefined}>
      <div className="flex h-full min-h-0 overflow-hidden">
        <Sidebar />
        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-14 shrink-0 items-center justify-between gap-4 border-b border-border px-4 lg:px-6">
            <h1 className="truncate text-lg font-semibold tracking-tight">{t(`nav.${section}`)}</h1>
            <div className="flex shrink-0 items-center gap-2">
              <VoiceHeaderToggle />
              <TaskCenter />
              <CurrentAccount />
            </div>
          </header>
          <UpdateBanner />
          <main className="relative min-h-0 flex-1 overflow-x-hidden overflow-y-auto" data-testid="main">
            <AnimatePresence mode="wait" initial={false}>
              <motion.div
                key={section}
                {...screen}
                className="mx-auto w-full max-w-5xl p-4 lg:p-6 2xl:max-w-6xl"
              >
                <Screen />
              </motion.div>
            </AnimatePresence>
          </main>
        </div>
      </div>
      <Toaster />
    </MotionConfig>
  );
}
