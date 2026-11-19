import { CalendarDayIcon, ListIcon, MailboxIcon } from "./Icons";

export type Tab = "inbox" | "today" | "views";

export function TabBar({
  tab,
  day,
  onSelect,
}: {
  tab: Tab;
  day: string | number;
  onSelect: (t: Tab) => void;
}) {
  const btn = (name: Tab, label: string, icon: preact.ComponentChildren) => (
    <button
      role="tab"
      type="button"
      aria-label={label}
      aria-selected={tab === name}
      class={tab === name ? "tab selected" : "tab"}
      onClick={() => onSelect(name)}
    >
      {icon}
    </button>
  );
  return (
    <nav class="tabbar" role="tablist">
      {btn("inbox", "Inbox", <MailboxIcon />)}
      {btn("today", "Today", <CalendarDayIcon day={day} />)}
      {btn("views", "Views", <ListIcon />)}
    </nav>
  );
}

/** Day of month from a `YYYY-MM-DD` date; empty before the first view arrives. */
export function dayOf(today: string | undefined): string {
  if (!today) return "";
  return String(parseInt(today.slice(8, 10), 10) || "");
}
