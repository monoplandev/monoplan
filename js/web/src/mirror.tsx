// Entry of the macOS app's window (`mirror.html`): the browser client's
// source over a mirror doc (`spec/mac-window-plan.md`). The browser
// entry is `index.tsx`; nothing here is loaded by it, so the browser
// bundle is untouched by the app's existence. (Named for the mirror
// rather than `app.tsx`, which would collide with `App.tsx` on a
// case-insensitive filesystem.)

import { render } from "solid-js/web";
import { AppI18nProvider } from "./i18n.tsx";
import { MirrorApp } from "./MirrorApp.tsx";
import "./styles.css";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
render(
  () => (
    <AppI18nProvider>
      <MirrorApp />
    </AppI18nProvider>
  ),
  root,
);
