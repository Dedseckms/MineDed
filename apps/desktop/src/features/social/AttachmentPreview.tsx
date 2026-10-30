import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api, unwrap } from "../../api/client";
import type { SocialAttachment } from "../../bindings";
import { Button } from "../../components/ui/Button";
import { PhotoViewer } from "../../components/ui/PhotoViewer";
import { ErrorMessage } from "../../components/ui/feedback";

/**
 * Native authenticated, bounded thumbnail. Never expose protected hub URLs to
 * the WebView. Clicking the thumbnail opens the full-screen viewer; other
 * attachment kinds keep their save button in the bubble.
 */
export function AttachmentPreview({
  attachment,
  onSave,
}: {
  attachment: SocialAttachment;
  /** Saves the original file from the viewer toolbar. */
  onSave: (attachment: SocialAttachment) => void;
}) {
  const { t } = useTranslation();
  const [visible, setVisible] = useState(false);
  const [viewer, setViewer] = useState(false);
  const preview = useQuery({
    queryKey: ["social", "preview", attachment.id],
    queryFn: async () => unwrap(await api.socialAttachmentPreview(attachment.id)),
    enabled: visible,
    staleTime: Infinity,
  });
  const safe = preview.data?.startsWith("data:image/webp;base64,")
    ? preview.data
    : null;
  return (
    <div>
      <Button size="sm" variant="ghost" onClick={() => setVisible(!visible)}>
        {t(visible ? "social.hidePreview" : "social.showPreview")}
      </Button>
      {visible ? (
        <>
          <ErrorMessage error={preview.error} />
          {preview.isPending ? (
            <p role="status">{t("common.loading")}</p>
          ) : null}
          {safe ? (
            <>
              <button
                type="button"
                aria-label={t("social.openFullscreen")}
                title={t("social.openFullscreen")}
                className="block cursor-zoom-in"
                onClick={() => setViewer(true)}
              >
                <img
                  src={safe}
                  alt={t("social.attachment")}
                  className="h-16 max-w-full rounded object-contain"
                />
              </button>
              <PhotoViewer
                src={viewer ? safe : null}
                alt={attachment.fileName}
                onClose={() => setViewer(false)}
                onSave={() => onSave(attachment)}
              />
            </>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
