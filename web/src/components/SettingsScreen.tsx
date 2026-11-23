import type { SettingsGetResult } from "../gen/protocol";
import type { Push } from "../push";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const pad = (n: number) => String(n).padStart(2, "0");

/** A last-sync instant in phone-local time: `10:42` today, `21 Nov 10:42` on another day. */
export function lastSynced(iso: string | null | undefined, now: Date = new Date()): string {
  if (!iso) return "Never";
  const t = new Date(iso);
  if (Number.isNaN(t.getTime())) return "Never";
  const time = `${pad(t.getHours())}:${pad(t.getMinutes())}`;
  return t.toDateString() === now.toDateString() ? time : `${t.getDate()} ${MONTHS[t.getMonth()]} ${time}`;
}

function Row({ label, value }: { label: string; value?: string | number }) {
  return (
    <li class="settings-row">
      <span class="view-name">{label}</span>
      {value !== undefined && <span class="settings-value">{value}</span>}
    </li>
  );
}

/**
 * The Settings screen (issue #167): the Reminders switch, then what the daemon reports about
 * itself. `settings` is undefined before the first answer and null from a daemon without
 * `settings.get`.
 */
export function SettingsScreen({ settings, push }: { settings: SettingsGetResult | null | undefined; push?: Push }) {
  return (
    <>
      {push && <RemindersRow push={push} />}
      {settings === null && <p class="reminders-hint">Daemon info unavailable</p>}
      {settings && <SyncSection settings={settings} />}
      {settings && (
        <section class="group">
          <h2>About</h2>
          <ul class="card">
            <Row label="Twydo" value={settings.version} />
            <Row label="Protocol" value={settings.protocol} />
          </ul>
        </section>
      )}
    </>
  );
}

function SyncSection({ settings }: { settings: SettingsGetResult }) {
  const { sync, server, reminders } = settings;
  return (
    <section class="group">
      <h2>Sync</h2>
      <ul class="card">
        {!sync && !server && <Row label="Sync is not configured" />}
        {sync && (
          <li class="settings-row settings-status">
            <span class={sync.attention ? "settings-headline attention" : "settings-headline"}>{sync.headline}</span>
            {sync.detail && <p>{sync.detail}</p>}
          </li>
        )}
        {sync && <Row label="Last synced" value={lastSynced(sync.last_success)} />}
        {server && <Row label="Server" value={server} />}
        {reminders && (
          <li class="settings-row settings-status">
            <span class="settings-headline attention">Reminders are failing</span>
            <p>{reminders.error}</p>
          </li>
        )}
      </ul>
    </section>
  );
}

/** The Reminders switch (spec §8.2); hidden where push cannot work at all. */
function RemindersRow({ push }: { push: Push }) {
  const state = push.state.value;
  if (state === "unsupported") return null;
  const hint =
    state === "install"
      ? "Add Twydo to your Home Screen to get reminders"
      : state === "denied"
        ? "Allow notifications for Twydo in Settings"
        : undefined;
  const on = state === "on";
  return (
    <section class="group">
      <ul class="card">
        <li class="reminders-row">
          <span class="view-name">Reminders</span>
          <button
            type="button"
            role="switch"
            class="switch"
            aria-label="Reminders"
            aria-checked={on}
            disabled={hint !== undefined || state === "busy"}
            onClick={() => void (on ? push.disable() : push.enable())}
          />
        </li>
      </ul>
      {hint && <p class="reminders-hint">{hint}</p>}
      {push.error.value && <p class="reminders-hint">Could not turn reminders off; try again</p>}
    </section>
  );
}
