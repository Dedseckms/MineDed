import { useState } from "react";

import type { ProjectSummary } from "../../bindings";

const HUES = [160, 200, 260, 20, 330, 90];

/** Project icon with a lettered fallback (no icon / failed to load). */
export function ProjectIcon({ project, size = "md" }: { project: ProjectSummary; size?: "md" | "lg" }) {
  const [failed, setFailed] = useState(false);
  const cls = size === "lg" ? "size-16 text-2xl" : "size-12 text-lg";
  if (project.iconUrl && !failed) {
    return (
      <img
        src={project.iconUrl}
        alt=""
        loading="lazy"
        className={`${cls} shrink-0 rounded-lg bg-surface-2 object-cover`}
        onError={() => setFailed(true)}
      />
    );
  }
  const hue = HUES[project.title.length % HUES.length] ?? 160;
  return (
    <span
      aria-hidden
      className={`${cls} grid shrink-0 place-items-center rounded-lg font-semibold text-white`}
      style={{ background: `hsl(${hue} 45% 40%)` }}
    >
      {project.title.slice(0, 1).toUpperCase()}
    </span>
  );
}
