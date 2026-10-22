import { AppShell } from "./features/shell/AppShell";
import { OverlayApp } from "./features/voice/OverlayApp";
import { isOverlayRoute } from "./lib/overlay";

/** `#/overlay` renders the compact voice overlay (separate always-on-top window). */
export function App() {
  return isOverlayRoute(window.location.hash) ? <OverlayApp /> : <AppShell />;
}
