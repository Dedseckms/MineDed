import { useEffect, useState } from "react";

const SEARCH_DEBOUNCE_MS = 250;

/**
 * Debounced copy of a search field. `query` stays empty for a blank term and
 * after `reset()` (e.g. when the tab closes), which also clears the field.
 */
export function useDebouncedSearch(initial = "", delay = SEARCH_DEBOUNCE_MS) {
  const [search, setSearch] = useState(initial);
  const [query, setQuery] = useState("");
  useEffect(() => {
    const value = search.trim();
    if (!value) {
      setQuery("");
      return;
    }
    const timer = setTimeout(() => setQuery(value), delay);
    return () => clearTimeout(timer);
  }, [search, delay]);
  const reset = () => {
    setSearch("");
    setQuery("");
  };
  return { search, setSearch, query, reset };
}
