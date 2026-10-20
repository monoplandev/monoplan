import { render } from "solid-js/web";
import { BrowserTabGate } from "./BrowserTabGate.tsx";
import { AppI18nProvider } from "./i18n.tsx";
import { captureInviteFromLocation } from "./invite.ts";
import "./styles.css";

// A sharing link's secret rides in the fragment. Park it and strip it
// before anything mounts, so neither the router nor history sees it
// (`invite.ts`).
captureInviteFromLocation();

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
render(
  () => (
    <AppI18nProvider>
      <BrowserTabGate />
    </AppI18nProvider>
  ),
  root,
);
