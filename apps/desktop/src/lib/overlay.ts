// Compact always-on-top voice overlay (separate Tauri window on `#/overlay`).
import { isMock } from "../api/client";

export const OVERLAY_LABEL = "voice-overlay";
export const OVERLAY_HASH = "#/overlay";

export const isOverlayRoute = (hash: string): boolean => hash === OVERLAY_HASH || hash.startsWith(`${OVERLAY_HASH}?`);

/** Window options of the overlay (exported for tests). */
export const OVERLAY_WINDOW = {
  url: `index.html${OVERLAY_HASH}`,
  width: 260,
  height: 300,
  minWidth: 200,
  minHeight: 140,
  x: 24,
  y: 24,
  alwaysOnTop: true,
  decorations: false,
  transparent: true,
  shadow: false,
  resizable: true,
  skipTaskbar: true,
  focus: false,
  visibleOnAllWorkspaces: true,
} as const;

/**
 * Opens the overlay, or closes it when it is already open. Returns whether it
 * is open afterwards. In mock mode (plain browser) a popup is used instead.
 */
export async function toggleOverlay(title: string): Promise<boolean> {
  if (isMock) {
    window.open(OVERLAY_HASH, OVERLAY_LABEL, `width=${OVERLAY_WINDOW.width},height=${OVERLAY_WINDOW.height}`);
    return true;
  }
  const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
  const existing = await WebviewWindow.getByLabel(OVERLAY_LABEL);
  if (existing) {
    await existing.close();
    return false;
  }
  await new Promise<void>((resolve, reject) => {
    const win = new WebviewWindow(OVERLAY_LABEL, { ...OVERLAY_WINDOW, title });
    void win.once("tauri://created", () => resolve());
    void win.once<unknown>("tauri://error", (e) => reject(new Error(String(e.payload))));
  });
  return true;
}

/** Closes the overlay from inside it. */
export async function closeOverlaySelf(): Promise<void> {
  if (isMock) {
    window.close();
    return;
  }
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().close();
}
