import { useMutation } from "@tanstack/react-query";
import { useEffect } from "react";

import type { UpdateInfo } from "../bindings";
import { api, unwrap } from "../api/client";
import { subscribe } from "../api/events";
import { useSettings, useStartTask } from "../api/queries";
import { errorView, toAppError } from "../lib/errors";
import { useTaskView } from "../stores/taskStore";
import { useToastStore } from "../stores/toastStore";
import { useUpdateStore } from "../stores/updateStore";

/** Runs `update_check` and stores the result (`null` = up to date). */
export async function checkForUpdate(): Promise<UpdateInfo | null> {
  const info = unwrap(await api.updateCheck());
  useUpdateStore.getState().setAvailable(info);
  return info;
}

/**
 * App-wide update wiring: listens to `update://available` and, when
 * `settings.autoUpdate` is on, runs one silent `update_check` at startup.
 * The backend's own startup check may emit its event before the UI
 * subscribes, so the explicit check is what guarantees the banner.
 */
export function useUpdateSync(): void {
  const settings = useSettings();
  const autoUpdate = settings.data?.autoUpdate;

  useEffect(() => subscribe("updateAvailable", (info) => useUpdateStore.getState().setAvailable(info)), []);

  useEffect(() => {
    if (autoUpdate !== true) return;
    const store = useUpdateStore.getState();
    if (store.autoChecked) return;
    store.markAutoChecked();
    // Offline / blocked endpoints at startup are not worth a notification.
    checkForUpdate().catch(() => undefined);
  }, [autoUpdate]);
}

/** Manual "Check for updates": un-hides a postponed banner when an update is found. */
export function useCheckUpdate() {
  return useMutation({
    mutationFn: checkForUpdate,
    onSuccess: (info) => {
      if (info) useUpdateStore.getState().undismiss();
    },
  });
}

/**
 * Starts `update_install` (shown in the task center). On success the backend
 * restarts the app; failures (except cancel) raise a toast.
 */
export function useInstallUpdate() {
  const installTask = useUpdateStore((s) => s.installTask);
  const task = useTaskView(installTask);
  const start = useStartTask<void>({
    run: (_vars, channel) => api.updateInstall(channel),
    onFinished: (error) => {
      if (error && error.code !== "cancelled")
        useToastStore.getState().push({
          tone: "danger",
          messageKey: "updates.installFailed",
          detailKey: errorView(error).messageKey,
          dedupeKey: "update:install",
          error,
        });
    },
  });
  const install = () =>
    start.mutate(undefined, {
      onSuccess: (id) => useUpdateStore.getState().setInstallTask(id),
    });
  const running = start.isPending || task?.status === "running";
  return {
    install,
    running,
    task,
    /** Error of the command itself (the task's own failure is in `task.error`). */
    error: start.error ? toAppError(start.error) : null,
  };
}
