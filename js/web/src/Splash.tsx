// The full-page wait: the wordmark at the exact centre of the viewport,
// standing in for the app while it boots (session restore, the tab gate,
// the doc load).
import { useAppI18n } from "./i18n.tsx";

const WORDMARK_URL = "/monoplan-logo.png";

export function Splash() {
  const { m } = useAppI18n();
  return (
    <div class="splash" role="status" aria-label={m().common.loading}>
      <img class="splash-wordmark" src={WORDMARK_URL} alt="" width="840" height="208" />
    </div>
  );
}
