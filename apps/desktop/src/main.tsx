import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "./i18n";
import "./styles.css";
import { App } from "./App";
import { api, isDemo, isMock, loadMocks } from "./api/client";
import {
  INSTANCE_TABS,
  type InstanceTab,
  LIBRARY_TABS,
  type LibraryTab,
  SECTIONS,
  type Section,
  useNavStore,
} from "./stores/navStore";

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
});

// Mock mode only (browser dev): `?section=vpn` opens a section directly, for layout checks;
// `&library=catalog`, `&instance=<id>&tab=browse` open library/instance tabs; `&demo=1` → showcase.
if (isMock) {
  const q = new URLSearchParams(window.location.search);
  const nav = useNavStore.getState();
  const section = q.get("section");
  if (section && (SECTIONS as readonly string[]).includes(section)) nav.go(section as Section);
  const library = q.get("library");
  if (library && (LIBRARY_TABS as readonly string[]).includes(library)) nav.setLibraryTab(library as LibraryTab);
  const instance = q.get("instance");
  const tab = q.get("tab");
  if (instance)
    nav.showInstance(instance, tab && (INSTANCE_TABS as readonly string[]).includes(tab) ? (tab as InstanceTab) : undefined);
}

// The showcase state is seeded before the first render so no query sees the defaults.
const ready: Promise<void> = isDemo
  ? loadMocks()
      .then((m) => {
        const params = m.parseDemoParams(window.location.search);
        return params ? m.runDemo(api, m.mockBus, params) : undefined;
      })
      .catch(() => undefined)
  : Promise.resolve();

const root = document.getElementById("root");
if (root) void ready.then(() => {
  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </StrictMode>,
  );
});
