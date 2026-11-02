import { useTranslation } from "react-i18next";

import type { ContentKind, ContentSourceKind } from "../../bindings";
import { Icon } from "../../components/ui/Icon";
import { projectPageUrl } from "../../lib/content";

function siteName(source: ContentSourceKind): string {
  return source === "curseForge" ? "CurseForge" : "Modrinth";
}
/**
 * Link to the project page on the provider's site (label is the site name).
 * The opener plugin is not wired into the desktop build yet, so plain
 * `<a target="_blank">` hands external http(s) URLs to the system browser.
 */
export function ProjectPageLink({
  source,
  projectId,
  kind,
  slug,
  iconOnly = false,
  className = "",
}: {
  source: ContentSourceKind;
  projectId: string | null;
  kind: ContentKind;
  /** Human-readable page slug; falls back to the project id. */
  slug?: string | null;
  /** Icon-only variant for tight rows (labelled for screen readers). */
  iconOnly?: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const site = siteName(source);
  const href = projectPageUrl(source, projectId, kind, slug);
  if (!href) return null;
  const label = t("content.openOnSite", { site });
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer noopener"
      aria-label={iconOnly ? label : undefined}
      title={label}
      data-testid="project-page-link"
      className={
        iconOnly
          ? `inline-grid size-7 shrink-0 cursor-pointer place-items-center rounded-lg text-muted transition-colors hover:bg-surface-2 hover:text-text ${className}`
          : `inline-flex min-w-0 cursor-pointer items-center gap-1 text-xs text-accent underline-offset-2 hover:underline ${className}`
      }
    >
      <Icon name="external" className="size-3.5 shrink-0" />
      {iconOnly ? null : <span className="truncate">{label}</span>}
    </a>
  );
}
