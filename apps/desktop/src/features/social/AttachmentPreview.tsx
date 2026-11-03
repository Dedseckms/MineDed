import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { api, unwrap } from "../../api/client";
import type { SocialAttachment } from "../../bindings";
import { PhotoViewer } from "../../components/ui/PhotoViewer";

/** Preview side of a single image: 200–320 px, so a thumbnail reads at a glance. */
const SINGLE_THUMB = "h-56 w-56 max-w-full";

/** Protected hub URLs never reach the WebView: previews arrive as bounded data URLs. */
function usePreview(attachment: SocialAttachment) {
  const query = useQuery({
    queryKey: ["social", "preview", attachment.id],
    queryFn: async () => unwrap(await api.socialAttachmentPreview(attachment.id)),
    staleTime: Infinity,
  });
  const src = query.data?.startsWith("data:image/webp;base64,")
    ? query.data
    : null;
  return { src, pending: query.isPending, failed: !!query.error };
}

export function isImage(attachment: SocialAttachment): boolean {
  return attachment.mime.startsWith("image/");
}

/**
 * Full-size original, fetched only once the viewer opens; the bounded preview
 * shows first so the dialog is never empty. Failed load keeps the preview.
 */
function useOriginal(attachment: SocialAttachment, open: boolean) {
  const query = useQuery({
    queryKey: ["social", "original", attachment.id],
    queryFn: async () => unwrap(await api.socialAttachmentOriginal(attachment.id)),
    enabled: open,
    staleTime: Infinity,
    retry: false,
  });
  return query.data?.startsWith("data:image/") ? query.data : null;
}

/**
 * Inline thumbnail with an immediate preview; the save button lives only in the
 * full-screen viewer, never in the bubble.
 */
export function ImageThumb({
  attachment,
  onSave,
  className = SINGLE_THUMB,
}: {
  attachment: SocialAttachment;
  onSave: (attachment: SocialAttachment) => void;
  className?: string;
}) {
  const { t } = useTranslation();
  const [viewer, setViewer] = useState(false);
  const { src, pending } = usePreview(attachment);
  const original = useOriginal(attachment, viewer);
  return (
    <div
      className={`relative overflow-hidden rounded-lg bg-surface ${className}`}
    >
      {src ? (
        <button
          type="button"
          aria-label={t("social.openFullscreen")}
          title={attachment.fileName}
          className="block size-full cursor-zoom-in"
          onClick={() => setViewer(true)}
        >
          <img
            src={src}
            alt={t("social.attachment")}
            draggable={false}
            className="size-full object-cover"
          />
        </button>
      ) : (
        <p className="grid size-full place-items-center p-1 text-center text-[11px] break-all text-muted">
          {t(pending ? "common.loading" : "social.previewUnavailable")}
        </p>
      )}
      <PhotoViewer
        src={viewer ? (original ?? src) : null}
        alt={attachment.fileName}
        onClose={() => setViewer(false)}
        onSave={() => onSave(attachment)}
      />
    </div>
  );
}

/**
 * Image previews of one message: a plain thumbnail for a single picture, a grid
 * for a run of them (2 columns, 3 once there are more than four).
 */
export function AttachmentGrid({
  attachments,
  onSave,
}: {
  attachments: SocialAttachment[];
  onSave: (attachment: SocialAttachment) => void;
}) {
  const columns =
    attachments.length === 1 ? 1 : attachments.length <= 4 ? 2 : 3;
  return (
    <div
      className="grid w-fit max-w-full gap-1"
      style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}
    >
      {attachments.map((attachment) => (
        <ImageThumb
          key={attachment.id}
          attachment={attachment}
          onSave={onSave}
          className={columns === 1 ? SINGLE_THUMB : "aspect-square w-full"}
        />
      ))}
    </div>
  );
}
