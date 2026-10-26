import { useEffect } from "react";

import { isMock } from "../api/client";
import { parseDeepLink } from "../lib/deepLink";
import { useNavStore } from "../stores/navStore";

/**
 * Routes a deep link. `open` needs nothing here: the single-instance plugin
 * already shows and focuses the window. Returns whether the link was known.
 */
export function handleDeepLink(url: string): boolean {
  const link = parseDeepLink(url);
  if (!link) return false;
  const nav = useNavStore.getState();
  if (link.kind === "join") nav.openJoin(link.code);
  else if (link.kind === "invite") nav.openInvite(link.code);
  return true;
}

/**
 * Deep links of the main window: the URL the app was started with
 * (`getCurrent`) and links opened while running (`onOpenUrl`). In mock mode
 * `?link=<url>` stands in for the start URL.
 */
export function useDeepLinks(): void {
  useEffect(() => {
    if (isMock) {
      const link = new URLSearchParams(window.location.search).get("link");
      if (link) handleDeepLink(link);
      return;
    }
    let stop: (() => void) | null = null;
    let disposed = false;
    void (async () => {
      try {
        const dl = await import("@tauri-apps/plugin-deep-link");
        const start = await dl.getCurrent();
        if (disposed) return;
        start?.forEach(handleDeepLink);
        const off = await dl.onOpenUrl((urls) => urls.forEach(handleDeepLink));
        if (disposed) off();
        else stop = off;
      } catch {
        // Plugin unavailable (e.g. web build): deep links are optional.
      }
    })();
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);
}
