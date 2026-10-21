import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";

import { api, unwrap } from "../api/client";
import { subscribe } from "../api/events";
import { qk } from "../api/queries";
import { useGameStore } from "../stores/gameStore";
import { useTaskStore } from "../stores/taskStore";

/**
 * App-wide subscriptions: task center (`tasks_list` + `tasks://changed`),
 * game state/log and account changes. Mounted once in the shell.
 */
export function useBackendSync(): void {
  const qc = useQueryClient();
  const syncSummaries = useTaskStore((s) => s.syncSummaries);
  const applyState = useGameStore((s) => s.applyState);
  const appendLog = useGameStore((s) => s.appendLog);

  const tasks = useQuery({ queryKey: qk.tasks, queryFn: async () => unwrap(await api.tasksList()) });

  useEffect(() => {
    if (tasks.data) syncSummaries(tasks.data);
  }, [tasks.data, syncSummaries]);

  useEffect(() => {
    const offs = [
      subscribe("tasksChanged", (list) => {
        qc.setQueryData(qk.tasks, list);
      }),
      subscribe("instanceState", (p) => {
        applyState(p);
        qc.setQueryData(qk.instanceState(p.id), p.state);
        // lastPlayed / playtime change around launches.
        if (p.state === "running" || p.state === "stopped" || p.state === "crashed")
          void qc.invalidateQueries({ queryKey: qk.instances });
      }),
      subscribe("instanceLog", appendLog),
      subscribe("accountChanged", (list) => {
        qc.setQueryData(qk.accounts, list);
      }),
    ];
    return () => offs.forEach((off) => off());
  }, [qc, applyState, appendLog]);
}
