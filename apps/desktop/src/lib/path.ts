/** Last segment of a Windows or POSIX path (`data:` URLs and empty input → `""`). */
export function fileName(path: string): string {
  if (path.startsWith("data:")) return "";
  const parts = path.split(/[\\/]/).filter((s) => s.length > 0);
  return parts.at(-1) ?? "";
}
