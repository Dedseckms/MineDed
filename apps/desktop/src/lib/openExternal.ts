import { openPath, openUrl } from "@tauri-apps/plugin-opener";

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

/**
 * Opens a folder of an instance in the system file manager; `sub` appends a
 * child folder (e.g. `mods`) when the directory exists.
 */
export async function openFolder(gameDir: string | undefined, sub = ""): Promise<void> {
  if (!gameDir) return;
  const trimmed = gameDir.replace(/[\\/]+$/, "");
  const sep = trimmed.includes("\\") ? "\\" : "/";
  const path = sub ? `${trimmed}${sep}${sub}` : trimmed;
  try {
    await openPath(path);
  } catch {
    // Outside Tauri (tests, plain browser) there is nothing to open.
  }
}
