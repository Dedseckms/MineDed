import type { SocialAttachment } from "../../bindings";

/** Splits a file name into a lower-case extension without the dot. */
function extensionOf(name: string): string {
  const match = /\.([a-z0-9]{1,8})$/i.exec(name.trim());
  return match ? match[1]!.toLowerCase() : "";
}

/** Usual extension of a mime type, used only for non-image files. */
const MIME_EXT: Record<string, string> = {
  "text/plain": "txt",
  "application/zip": "zip",
  "application/x-mrpack": "mrpack",
  "application/java-archive": "jar",
  "application/json": "json",
};

/**
 * File name to offer in the "save as" dialog. The attachment keeps the original
 * name, so its extension wins over the transcoded `mime`: a photo sent as
 * `shot.png` and stored as WebP is offered as `shot.png`, never `shot.webp`.
 * An unknown picture falls back to `.png`; other files use their mime type.
 */
export function saveFileName(attachment: SocialAttachment): string {
  const base = attachment.fileName.trim().replace(/[\\/:*?"<>|]/g, "_").replace(/^\.+/, "");
  if (extensionOf(base)) return base;
  const fallback = attachment.mime.startsWith("image/")
    ? "png"
    : (MIME_EXT[attachment.mime] ?? "bin");
  const stem = base || attachment.id.replace(/[\\/:*?"<>|]/g, "_") || "file";
  return `${stem}.${fallback}`;
}
