import { useEffect, useRef, useState } from "react";

/**
 * Local value for a slider that is committed to the backend `ms` after the
 * last change (no IPC per pixel). Follows `value` from outside while idle.
 */
export function useDraftValue<T>(value: T, commit: (v: T) => void, ms = 250): [T, (v: T) => void] {
  const [draft, setDraft] = useState(value);
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);
  const commitRef = useRef(commit);
  commitRef.current = commit;

  useEffect(() => {
    if (pending.current === null) setDraft(value);
  }, [value]);

  const latest = useRef(value);
  // Flush a pending change on unmount instead of dropping it.
  useEffect(
    () => () => {
      if (pending.current === null) return;
      clearTimeout(pending.current);
      pending.current = null;
      commitRef.current(latest.current);
    },
    [],
  );

  const update = (v: T) => {
    latest.current = v;
    setDraft(v);
    if (pending.current !== null) clearTimeout(pending.current);
    pending.current = setTimeout(() => {
      pending.current = null;
      commitRef.current(v);
    }, ms);
  };
  return [draft, update];
}
