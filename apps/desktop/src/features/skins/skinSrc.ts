import { convertFileSrc } from "@tauri-apps/api/core";

import type { Skin } from "../../bindings";

/**
 * Image source of a skin: the texture URL, or the locally cached PNG served
 * through Tauri's asset protocol when there is no URL (local-only skins).
 * Returns `null` when neither is usable.
 */
export function skinSrc(
  skin: Pick<Skin, "url" | "localPath">,
  toAsset: (path: string) => string = convertFileSrc,
): string | null {
  if (skin.url.trim() !== "") return skin.url;
  if (!skin.localPath) return null;
  try {
    return toAsset(skin.localPath);
  } catch {
    // Outside Tauri (browser mocks) there is no asset protocol.
    return null;
  }
}
