import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";

import { Button } from "../../components/ui/Button";
import { Icon } from "../../components/ui/Icon";
import { useMotionPrefs } from "../../hooks/useAppearance";
import { selectRunningCount, selectVisibleTasks, useTaskStore } from "../../stores/taskStore";
import { TaskProgress } from "../tasks/TaskProgress";

/** Header button + dropdown listing all backend tasks. */
export function TaskCenter() {
  const { t } = useTranslation();
  const { light } = useMotionPrefs();
  const [open, setOpen] = useState(false);
  const tasks = useTaskStore(useShallow(selectVisibleTasks));
  const running = useTaskStore(selectRunningCount);
  const dismissFinished = useTaskStore((s) => s.dismissFinished);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const hasFinished = tasks.some((x) => x.status !== "running");

  return (
    <div ref={rootRef} className="relative">
      <Button
        variant="ghost"
        size="sm"
        aria-expanded={open}
        aria-haspopup="dialog"
        onClick={() => setOpen((v) => !v)}
        icon={<Icon name="tasks" />}
      >
        {t("tasks.center")}
        {running > 0 ? (
          <span className="ml-1 inline-flex min-w-5 items-center justify-center rounded-full bg-accent px-1.5 text-xs text-accent-fg tabular-nums">
            {running}
          </span>
        ) : null}
      </Button>
      <AnimatePresence>
        {open ? (
          <motion.div
            role="dialog"
            aria-label={t("tasks.center")}
            initial={{ opacity: 0, y: -6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -6 }}
            transition={{ duration: 0.15 }}
            className={`absolute right-0 z-40 mt-2 flex max-h-[70vh] w-96 flex-col overflow-hidden rounded-xl border border-border shadow-2xl ${
              light ? "bg-surface" : "bg-surface/90 backdrop-blur-md"
            }`}
          >
            <div className="flex items-center justify-between border-b border-border px-4 py-3">
              <h2 className="text-sm font-semibold">{t("tasks.center")}</h2>
              <Button size="sm" variant="ghost" disabled={!hasFinished} onClick={dismissFinished}>
                {t("tasks.clearFinished")}
              </Button>
            </div>
            <div className="flex-1 overflow-y-auto">
              {tasks.length === 0 ? (
                <p className="px-4 py-8 text-center text-sm text-muted">{t("tasks.empty")}</p>
              ) : (
                <ul className="divide-y divide-border">
                  {tasks.map((task) => (
                    <li key={task.id} className="px-4 py-3">
                      <TaskProgress task={task} />
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}
