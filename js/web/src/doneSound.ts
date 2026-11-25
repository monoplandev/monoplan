// The sound played when an item is ticked done. A device preference,
// off by default, so localStorage like `agendaHideEmptyDays.ts`: only
// "on" is stored. Played by the routed actions (`sync/docs.ts`) on the
// user's own tick, so a close arriving by sync or redone by undo stays
// silent.
//
// The clip is `public/sounds/done.wav` (0.09 s, mono 16-bit PCM, which
// every browser decodes; the Apple apps ship the same file).
import { createSignal } from "solid-js";

const KEY = "monoplan.doneSound";
const SRC = "/sounds/done.wav";

function readDoneSoundPref(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

const [doneSoundPref, setDoneSoundSignal] = createSignal(readDoneSoundPref());
export { doneSoundPref };

let clip: HTMLAudioElement | null = null;

/** The one element every tick replays; made on first need so a device
 *  with the sound off never fetches the file. */
function load(): HTMLAudioElement | null {
  if (clip || typeof Audio === "undefined") return clip;
  clip = new Audio(SRC);
  clip.preload = "auto";
  return clip;
}

export function setDoneSoundPref(on: boolean): void {
  setDoneSoundSignal(on);
  try {
    if (on) localStorage.setItem(KEY, "1");
    else localStorage.removeItem(KEY);
  } catch {
    // Private-mode / quota failures: the choice just won't stick.
  }
  // Fetch now, so the first tick is not the one that waits on it.
  if (on) load();
}

if (doneSoundPref()) load();

/** Play the tick, if the device has it on. */
export function playDoneSound(): void {
  if (!doneSoundPref()) return;
  const audio = load();
  if (!audio) return;
  try {
    audio.currentTime = 0;
    // Rejects when the browser blocks playback (no user gesture yet).
    void audio.play().catch(() => {});
  } catch {}
}
