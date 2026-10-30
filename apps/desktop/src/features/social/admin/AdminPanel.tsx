import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { SocialUser } from "../../../bindings";
import { Icon } from "../../../components/ui/Icon";
import { Tabs } from "../../../components/ui/Tabs";
import { Badge } from "../../../components/ui/feedback";
import { isAdmin } from "../../../lib/social";
import { JournalTab } from "./JournalTab";
import { ModerationTab } from "./ModerationTab";
import { ServerTab } from "./ServerTab";
import { UsersTab } from "./UsersTab";

type AdminTab = "moderation" | "users" | "server" | "journal";

const TABS: AdminTab[] = ["moderation", "users", "server", "journal"];

/** Full-screen administration section inside the chat tab; admins only. */
export function AdminPanel({ self }: { self: SocialUser }) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<AdminTab>("moderation");
  if (!isAdmin(self.role)) return null;
  return (
    <section
      aria-label={t("social.admin.title")}
      className="flex min-h-0 flex-1 flex-col gap-2"
    >
      <header className="flex flex-wrap items-center gap-2">
        <span className="grid size-8 place-items-center rounded-lg bg-accent/15 text-accent">
          <Icon name="shield" />
        </span>
        <div className="min-w-0 flex-1">
          <h3 className="text-lg font-semibold">{t("social.admin.title")}</h3>
          <p className="text-xs text-muted">{t("social.admin.subtitle")}</p>
        </div>
        <Badge tone="accent">{t("social.role.admin")}</Badge>
      </header>
      <Tabs
        value={tab}
        tabs={TABS.map((value) => ({
          value,
          label: t(`social.admin.tabs.${value}`),
        }))}
        onChange={setTab}
        ariaLabel={t("social.admin.tabsLabel")}
        idPrefix="admin"
      />
      <div
        role="tabpanel"
        id={`admin-panel-${tab}`}
        aria-labelledby={`admin-${tab}`}
        className="flex min-h-0 flex-1 flex-col"
      >
        {tab === "moderation" ? <ModerationTab /> : null}
        {tab === "users" ? <UsersTab /> : null}
        {tab === "server" ? <ServerTab /> : null}
        {tab === "journal" ? <JournalTab /> : null}
      </div>
    </section>
  );
}
