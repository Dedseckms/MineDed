import { openUrl } from "@tauri-apps/plugin-opener";

/** Opens an external http(s) link in the system browser; webviews ignore `target="_blank"`. */
export async function openExternal(url: string): Promise<void> {
  if (!/^https?:\/\//i.test(url)) return;
  try {
    await openUrl(url);
  } catch {
    // Outside Tauri (tests, plain browser) fall back to a new tab.
    window.open(url, "_blank", "noopener,noreferrer");
  }
}
