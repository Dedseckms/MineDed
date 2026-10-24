import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "./i18n";
import "./styles.css";
import { App } from "./App";
import { isMock } from "./api/client";
import { SECTIONS, type Section, useNavStore } from "./stores/navStore";

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
});

// Mock mode only (browser dev): `?section=vpn` opens a section directly, for layout checks.
if (isMock) {
  const section = new URLSearchParams(window.location.search).get("section");
  if (section && (SECTIONS as readonly string[]).includes(section)) useNavStore.getState().go(section as Section);
}

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </StrictMode>,
  );
}
