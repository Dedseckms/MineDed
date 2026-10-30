import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api, unwrap } from "../../api/client";
import { Button } from "../../components/ui/Button";
import { ErrorMessage } from "../../components/ui/feedback";

/** Native authenticated, bounded thumbnail. Never expose protected hub URLs to the WebView. */
export function AttachmentPreview({ id }: { id: string }) {
  const { t } = useTranslation();
  const [visible, setVisible] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const preview = useQuery({
    queryKey: ["social", "preview", id],
    queryFn: async () => unwrap(await api.socialAttachmentPreview(id)),
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
            <button
              aria-label={t("social.expandPreview")}
              onClick={() => setExpanded(!expanded)}
            >
              <img
                src={safe}
                alt={t("social.attachment")}
                className={
                  expanded
                    ? "max-h-96 max-w-full rounded-lg object-contain"
                    : "h-16 max-w-full rounded object-contain"
                }
              />
            </button>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
