import type { RemindersSummary } from "../gen/protocol";

export function RemindersWarning({ reminders }: { reminders: RemindersSummary }) {
  return (
    <div class="sync-warning" role="alert">
      <strong>Reminders are failing</strong>
      <p>{reminders.error}</p>
    </div>
  );
}
