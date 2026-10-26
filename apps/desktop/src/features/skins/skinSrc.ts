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
  return skin.localPath ? localSkinSrc(skin.localPath, toAsset) : null;
}

/**
 * Image source of a local PNG (picked file, `RemoteSkin.localPath`). URLs
 * (`data:`, `http(s):`, used by the mocks) pass through unchanged.
 */
export function localSkinSrc(path: string, toAsset: (path: string) => string = convertFileSrc): string | null {
  const p = path.trim();
  if (p === "") return null;
  if (/^(?:data:|https?:|blob:)/i.test(p)) return p;
  try {
    return toAsset(p);
  } catch {
    // Outside Tauri (browser mocks) there is no asset protocol.
    return null;
  }
}
