import { useQuery } from "@tanstack/react-query";
import { motion } from "motion/react";
import { useTranslation } from "react-i18next";

import { api, isMock, unwrap } from "./api/client";

/** Placeholder shell. Replaced by ui in wave 1. */
export function App() {
  const { t } = useTranslation();
  const version = useQuery({ queryKey: ["appVersion"], queryFn: () => api.appVersion() });
  const instances = useQuery({ queryKey: ["instances"], queryFn: async () => unwrap(await api.instanceList()) });

  return (
    <main className="flex h-full flex-col items-center justify-center gap-4">
      <motion.h1
        className="text-3xl font-semibold"
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.3 }}
      >
        {t("app.title")}
      </motion.h1>
      <p className="text-[var(--color-muted)]">
        {t("app.version", { version: version.data ?? "…" })}
        {isMock ? ` · ${t("app.mock")}` : ""}
      </p>
      <p>{t("instances.count", { count: instances.data?.length ?? 0 })}</p>
    </main>
  );
}
