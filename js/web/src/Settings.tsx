import { Dialog } from "@kobalte/core/dialog";
import { DropdownMenu } from "@kobalte/core/dropdown-menu";
import { SegmentedControl } from "@kobalte/core/segmented-control";
import { Switch } from "@kobalte/core/switch";
import {
  createEffect,
  createSignal,
  For,
  onMount,
  Show,
  untrack,
} from "solid-js";
import { api, type Device } from "./api.ts";
import dotsVerticalSvg from "./icons/dots-vertical.svg?raw";
import type { Session } from "./Login.tsx";
import { useAppI18n } from "./i18n.tsx";
import { closeToItems, trackOverlay } from "./overlay.ts";
import { densityPref, setDensityPref, type ListDensity } from "./density.ts";
import type { ThemePreference } from "./theme.ts";
import {
  setTimeFormatPref,
  timeFormatPref,
  type TimeFormatPreference,
} from "./format.tsx";

type Section = "general" | "account" | "devices";

function DeviceNameEditor(props: {
  name: string;
  onSave: (name: string) => Promise<boolean>;
  onDone: () => void;
}) {
  const [value, setValue] = createSignal(props.name);
  const [saving, setSaving] = createSignal(false);
  let inputRef!: HTMLInputElement;
  let cancelled = false;

  onMount(() => {
    inputRef.focus();
    inputRef.select();
  });

  const commit = async () => {
    if (cancelled || saving()) return;
    const name = value().trim();
    if (!name) {
      inputRef.focus();
      return;
    }
    if (name === props.name) {
      props.onDone();
      return;
    }
    setSaving(true);
    if (await props.onSave(name)) props.onDone();
    else {
      setSaving(false);
      inputRef.focus();
    }
  };

  return (
    <form
      class="device-rename-form"
      onSubmit={(e) => {
        e.preventDefault();
        void commit();
      }}
    >
      <input
        ref={inputRef}
        class="device-rename-input"
        value={value()}
        disabled={saving()}
        onInput={(e) => setValue(e.currentTarget.value)}
        onBlur={() => void commit()}
        onKeyDown={(e) => {
          if (e.key !== "Escape") return;
          e.preventDefault();
          e.stopPropagation();
          cancelled = true;
          props.onDone();
        }}
      />
    </form>
  );
}

export function Settings(props: {
  open: boolean;
  onOpenChange: (b: boolean) => void;
  themePref: ThemePreference;
  onThemeChange: (pref: ThemePreference) => void;
  showListCounts: boolean;
  onShowListCountsChange: (show: boolean) => void;
  session: Session;
  logout: () => void;
}) {
  const { m, language, setLanguage, locale } = useAppI18n();
  trackOverlay(() => props.open);
  const [section, setSection] = createSignal<Section>("general");
  const [devices, setDevices] = createSignal<Device[] | null>(null);
  const [serverLastSeq, setServerLastSeq] = createSignal(0);
  const [devicesError, setDevicesError] = createSignal<string | null>(null);
  const [devicesLoading, setDevicesLoading] = createSignal(false);
  const [revoking, setRevoking] = createSignal<ReadonlySet<string>>(new Set());
  const [editingDeviceId, setEditingDeviceId] = createSignal<string | null>(null);

  async function renameDevice(id: string, name: string): Promise<boolean> {
    setDevicesError(null);
    try {
      await api.renameDevice(id, name);
      setDevices((prev) =>
        prev?.map((d) => (d.id === id ? { ...d, name } : d)) ?? prev,
      );
      return true;
    } catch (e) {
      setDevicesError(
        e instanceof Error ? e.message : m().settings.failedToRenameDevice,
      );
      return false;
    }
  }

  async function revokeDevice(id: string) {
    setDevicesError(null);
    setRevoking((prev) => new Set(prev).add(id));
    try {
      await api.deleteDevice(id);
      setDevices((prev) => (prev ? prev.filter((d) => d.id !== id) : prev));
    } catch (e) {
      setDevicesError(
        e instanceof Error ? e.message : m().settings.failedToRevokeDevice,
      );
    } finally {
      setRevoking((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  }

  async function loadDevices() {
    setDevicesError(null);
    setDevicesLoading(true);
    try {
      const res = await api.listDevices();
      setDevices(res.devices);
      setServerLastSeq(res.server_last_seq);
    } catch (e) {
      setDevicesError(
        e instanceof Error ? e.message : m().settings.failedToLoadDevices,
      );
    } finally {
      setDevicesLoading(false);
    }
  }

  // Refetch every time the Devices section is entered while authenticated.
  // `devicesLoading` is read untracked — it's a guard against stomping an
  // in-flight fetch, not a trigger; tracking it would self-loop when
  // loadDevices flips it back to false.
  createEffect(() => {
    if (props.open && section() === "devices" && !props.session.anonymous) {
      if (!untrack(devicesLoading)) {
        void loadDevices();
      }
    }
  });

  return (
    <Dialog open={props.open} onOpenChange={props.onOpenChange} modal>
      <Dialog.Portal>
        <Dialog.Overlay class="dialog-overlay" />
        <div class="dialog-positioner">
          <Dialog.Content class="settings-dialog" onCloseAutoFocus={closeToItems}>
            <aside class="settings-sidebar">
              <button
                type="button"
                class="settings-nav-item"
                data-active={section() === "general" ? "" : undefined}
                onClick={() => setSection("general")}
              >
                {m().settings.general}
              </button>
              <button
                type="button"
                class="settings-nav-item"
                data-active={section() === "account" ? "" : undefined}
                onClick={() => setSection("account")}
              >
                {m().settings.account}
              </button>
              <button
                type="button"
                class="settings-nav-item"
                data-active={section() === "devices" ? "" : undefined}
                onClick={() => setSection("devices")}
              >
                {m().settings.devices}
              </button>
            </aside>
            <section class="settings-content">
              <Dialog.CloseButton class="settings-close" aria-label={m().common.close}>
                <CloseIcon />
              </Dialog.CloseButton>
              <Show when={section() === "general"}>
                <h2 class="settings-section-title">{m().settings.general}</h2>
                <div class="settings-row">
                  <div class="settings-row-label">{m().settings.language}</div>
                  <SegmentedControl
                    class="theme-segmented"
                    aria-label={m().settings.language}
                    value={language()}
                    onChange={(value) => setLanguage(value as "es" | "en")}
                  >
                    <SegmentedControl.Indicator class="theme-segment-indicator" />
                    <SegmentedControl.Item value="es" class="theme-segment">
                      <SegmentedControl.ItemInput />
                      <SegmentedControl.ItemControl class="theme-segment-control">
                        <SegmentedControl.ItemLabel>
                          {m().settings.languageSpanish}
                        </SegmentedControl.ItemLabel>
                      </SegmentedControl.ItemControl>
                    </SegmentedControl.Item>
                    <SegmentedControl.Item value="en" class="theme-segment">
                      <SegmentedControl.ItemInput />
                      <SegmentedControl.ItemControl class="theme-segment-control">
                        <SegmentedControl.ItemLabel>
                          {m().settings.languageEnglish}
                        </SegmentedControl.ItemLabel>
                      </SegmentedControl.ItemControl>
                    </SegmentedControl.Item>
                  </SegmentedControl>
                </div>
                <div class="settings-row">
                  <div class="settings-row-label">{m().settings.theme}</div>
                  <SegmentedControl
                    class="theme-segmented"
                    aria-label={m().settings.theme}
                    value={props.themePref}
                    onChange={(value) =>
                      props.onThemeChange(value as ThemePreference)
                    }
                  >
                    <SegmentedControl.Indicator class="theme-segment-indicator" />
                    <SegmentedControl.Item value="auto" class="theme-segment">
                      <SegmentedControl.ItemInput />
                      <SegmentedControl.ItemControl class="theme-segment-control">
                        <SegmentedControl.ItemLabel>
                          {m().settings.auto}
                        </SegmentedControl.ItemLabel>
                      </SegmentedControl.ItemControl>
                    </SegmentedControl.Item>
                    <SegmentedControl.Item value="light" class="theme-segment">
                      <SegmentedControl.ItemInput />
                      <SegmentedControl.ItemControl
                        class="theme-segment-control"
                        aria-label={m().settings.light}
                      >
                        <SunIcon />
                      </SegmentedControl.ItemControl>
                    </SegmentedControl.Item>
                    <SegmentedControl.Item value="dark" class="theme-segment">
                      <SegmentedControl.ItemInput />
                      <SegmentedControl.ItemControl
                        class="theme-segment-control"
                        aria-label={m().settings.dark}
                      >
                        <MoonIcon />
                      </SegmentedControl.ItemControl>
                    </SegmentedControl.Item>
                  </SegmentedControl>
                </div>
                <div class="settings-row">
                  <div class="settings-row-label">{m().settings.density}</div>
                  <SegmentedControl
                    class="theme-segmented"
                    aria-label={m().settings.density}
                    value={densityPref()}
                    onChange={(value) => setDensityPref(value as ListDensity)}
                  >
                    <SegmentedControl.Indicator class="theme-segment-indicator" />
                    <SegmentedControl.Item value="standard" class="theme-segment">
                      <SegmentedControl.ItemInput />
                      <SegmentedControl.ItemControl class="theme-segment-control">
                        <SegmentedControl.ItemLabel>
                          {m().settings.densityStandard}
                        </SegmentedControl.ItemLabel>
                      </SegmentedControl.ItemControl>
                    </SegmentedControl.Item>
                    <SegmentedControl.Item value="compact" class="theme-segment">
                      <SegmentedControl.ItemInput />
                      <SegmentedControl.ItemControl class="theme-segment-control">
                        <SegmentedControl.ItemLabel>
                          {m().settings.densityCompact}
                        </SegmentedControl.ItemLabel>
                      </SegmentedControl.ItemControl>
                    </SegmentedControl.Item>
                  </SegmentedControl>
                </div>
                <div class="settings-row">
                  <div class="settings-row-label">{m().settings.timeFormat}</div>
                  <SegmentedControl
                    class="theme-segmented"
                    aria-label={m().settings.timeFormat}
                    value={timeFormatPref()}
                    onChange={(value) =>
                      setTimeFormatPref(value as TimeFormatPreference)
                    }
                  >
                    <SegmentedControl.Indicator class="theme-segment-indicator" />
                    <SegmentedControl.Item value="auto" class="theme-segment">
                      <SegmentedControl.ItemInput />
                      <SegmentedControl.ItemControl class="theme-segment-control">
                        <SegmentedControl.ItemLabel>
                          {m().settings.auto}
                        </SegmentedControl.ItemLabel>
                      </SegmentedControl.ItemControl>
                    </SegmentedControl.Item>
                    <SegmentedControl.Item value="12h" class="theme-segment">
                      <SegmentedControl.ItemInput />
                      <SegmentedControl.ItemControl class="theme-segment-control">
                        <SegmentedControl.ItemLabel>
                          {m().settings.timeFormat12}
                        </SegmentedControl.ItemLabel>
                      </SegmentedControl.ItemControl>
                    </SegmentedControl.Item>
                    <SegmentedControl.Item value="24h" class="theme-segment">
                      <SegmentedControl.ItemInput />
                      <SegmentedControl.ItemControl class="theme-segment-control">
                        <SegmentedControl.ItemLabel>
                          {m().settings.timeFormat24}
                        </SegmentedControl.ItemLabel>
                      </SegmentedControl.ItemControl>
                    </SegmentedControl.Item>
                  </SegmentedControl>
                </div>
                <div class="settings-row">
                  <div class="settings-row-label">
                    {m().settings.showListCounts}
                  </div>
                  <Switch
                    class="settings-switch"
                    aria-label={m().settings.showListCounts}
                    checked={props.showListCounts}
                    onChange={(checked) => props.onShowListCountsChange(checked)}
                  >
                    <Switch.Input class="settings-switch-input" />
                    <Switch.Control class="settings-switch-control">
                      <Switch.Thumb class="settings-switch-thumb" />
                    </Switch.Control>
                  </Switch>
                </div>
              </Show>
              <Show when={section() === "account"}>
                <h2 class="settings-section-title">{m().settings.account}</h2>
                <Show
                  when={props.session.anonymous}
                  fallback={
                    <>
                      <div class="settings-row">
                        <div class="settings-row-label">{m().settings.email}</div>
                        <div class="settings-row-value">
                          {props.session.email}
                        </div>
                      </div>
                      <div class="settings-row">
                        <button
                          type="button"
                          class="settings-logout"
                          onClick={() => props.logout()}
                        >
                          {m().nav.logOut}
                        </button>
                      </div>
                    </>
                  }
                >
                  <div class="settings-row">
                    <div class="settings-row-value">{m().settings.localOnlyAccount}</div>
                  </div>
                </Show>
              </Show>
              <Show when={section() === "devices"}>
                <h2 class="settings-section-title">{m().settings.devices}</h2>
                <Show
                  when={!props.session.anonymous}
                  fallback={
                    <div class="settings-row">
                      <div class="settings-row-value">{m().settings.loginToSeeDevices}</div>
                    </div>
                  }
                >
                  <Show when={devicesLoading() && devices() === null}>
                    <div class="settings-row">
                      <div class="settings-row-value">{m().common.loading}</div>
                    </div>
                  </Show>
                  <Show when={devicesError()}>
                    <div class="settings-row">
                      <div class="settings-row-value">{devicesError()}</div>
                    </div>
                  </Show>
                  <Show when={devices()}>
                    <ul class="device-list">
                      <For each={[...devices()!].sort((a, b) => b.last_seen_at - a.last_seen_at)}>
                        {(d) => (
                          <li class="device-row">
                            <div class="device-row-main">
                              <div class="device-name">
                                <Show
                                  when={editingDeviceId() === d.id}
                                  fallback={<span>{d.name}</span>}
                                >
                                  <DeviceNameEditor
                                    name={d.name}
                                    onSave={(name) => renameDevice(d.id, name)}
                                    onDone={() => setEditingDeviceId(null)}
                                  />
                                </Show>
                                <Show when={d.id === props.session.deviceId}>
                                  <span class="device-current-tag">
                                    {m().settings.thisDevice}
                                  </span>
                                </Show>
                              </div>
                              <div class="device-meta">
                                {m().settings.lastSeen} {formatRelative(d.last_seen_at, locale())}
                                <Show when={d.last_seen_ip}>
                                  {" · "}
                                  {d.last_seen_ip}
                                </Show>
                                {" · "}
                                {m().settings.deviceSeq(d.last_acked_seq, serverLastSeq())}
                              </div>
                            </div>
                            <Show when={editingDeviceId() !== d.id}>
                              <DropdownMenu placement="bottom-end" gutter={4}>
                                <DropdownMenu.Trigger
                                  class="device-menu-trigger"
                                  aria-label={`${m().settings.deviceActions}: ${d.name}`}
                                  innerHTML={dotsVerticalSvg}
                                />
                                <DropdownMenu.Portal>
                                  <DropdownMenu.Content class="dropdown-menu-content device-menu-content">
                                    <DropdownMenu.Item
                                      class="dropdown-menu-item"
                                      onSelect={() => {
                                        // Let the menu close and restore focus before
                                        // replacing its trigger with the rename input.
                                        requestAnimationFrame(() =>
                                          setEditingDeviceId(d.id),
                                        );
                                      }}
                                    >
                                      {m().settings.renameDevice}
                                    </DropdownMenu.Item>
                                    <Show when={d.id !== props.session.deviceId}>
                                      <DropdownMenu.Separator class="dropdown-menu-separator" />
                                      <DropdownMenu.Item
                                        class="dropdown-menu-item device-menu-revoke"
                                        disabled={revoking().has(d.id)}
                                        onSelect={() => {
                                          if (
                                            window.confirm(
                                              m().settings.revokeDeviceConfirm(d.name),
                                            )
                                          ) {
                                            void revokeDevice(d.id);
                                          }
                                        }}
                                      >
                                        {revoking().has(d.id)
                                          ? m().settings.revoking
                                          : m().settings.revoke}
                                      </DropdownMenu.Item>
                                    </Show>
                                  </DropdownMenu.Content>
                                </DropdownMenu.Portal>
                              </DropdownMenu>
                            </Show>
                          </li>
                        )}
                      </For>
                    </ul>
                  </Show>
                </Show>
              </Show>
            </section>
          </Dialog.Content>
        </div>
      </Dialog.Portal>
    </Dialog>
  );
}

function SunIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41" />
    </svg>
  );
}

function MoonIcon() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
    </svg>
  );
}

function formatRelative(ms: number, locale: string): string {
  const diffSec = Math.round((Date.now() - ms) / 1000);
  if (locale.startsWith("es")) return formatRelativeEs(ms, diffSec);
  return formatRelativeEn(ms, diffSec);
}

function formatRelativeEs(ms: number, diffSec: number): string {
  if (diffSec < 60) return "ahora mismo";
  if (diffSec < 3600) {
    const m = Math.floor(diffSec / 60);
    return `hace ${m} min`;
  }
  if (diffSec < 86400) {
    const h = Math.floor(diffSec / 3600);
    return `hace ${h} h`;
  }
  if (diffSec < 86400 * 7) {
    const d = Math.floor(diffSec / 86400);
    return `hace ${d} día${d === 1 ? "" : "s"}`;
  }
  return new Date(ms).toISOString().slice(0, 10);
}

function formatRelativeEn(ms: number, diffSec: number): string {
  if (diffSec < 60) return "Just now";
  if (diffSec < 3600) {
    const m = Math.floor(diffSec / 60);
    return `${m} minute${m === 1 ? "" : "s"} ago`;
  }
  if (diffSec < 86400) {
    const h = Math.floor(diffSec / 3600);
    return `${h} hour${h === 1 ? "" : "s"} ago`;
  }
  if (diffSec < 86400 * 7) {
    const d = Math.floor(diffSec / 86400);
    return `${d} day${d === 1 ? "" : "s"} ago`;
  }
  return new Date(ms).toISOString().slice(0, 10);
}

function CloseIcon() {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <line x1="18" y1="6" x2="6" y2="18" />
      <line x1="6" y1="6" x2="18" y2="18" />
    </svg>
  );
}
