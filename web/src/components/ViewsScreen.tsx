import type { ViewsListItem, ViewsListResult } from "../gen/protocol";
import type { ViewRef } from "../data";
import type { Push } from "../push";
import { CalendarDayIcon, ChevronRightIcon, ListIcon, MailboxIcon } from "./Icons";

const isItem = (e: ViewsListResult["entries"][number]): e is ViewsListItem => "view" in e;

function Glyph({ item, day }: { item: ViewsListItem; day: string | number }) {
  if (item.color) return <span class="view-bar" style={{ background: item.color }} aria-hidden="true" />;
  const icon =
    item.view.view === "inbox" ? (
      <MailboxIcon size={22} />
    ) : item.view.view === "today" ? (
      <CalendarDayIcon day={day} size={22} />
    ) : (
      <ListIcon size={22} />
    );
  return <span class="view-icon">{icon}</span>;
}

/** The Views tab's own screen: the sidebar entries, each opening a view (spec §8.2). */
export function ViewsScreen({
  list,
  day = "",
  onOpen,
  push,
}: {
  list: ViewsListResult;
  day?: string | number;
  onOpen: (ref: ViewRef) => void;
  push?: Push;
}) {
  // Consecutive items between headings form one card.
  const groups: { heading?: string; items: ViewsListItem[] }[] = [{ items: [] }];
  for (const e of list.entries) {
    if (isItem(e)) groups[groups.length - 1]!.items.push(e);
    else groups.push({ heading: e.heading, items: [] });
  }
  return (
    <>
      {groups
        .filter((g) => g.items.length > 0 || g.heading)
        .map((g, i) => (
          <section class="group" key={i}>
            {g.heading && <h2>{g.heading}</h2>}
            <ul class="card">
              {g.items.map((item) => (
                <li key={item.view.view + (item.view.list ?? "")}>
                  <button type="button" class="view-cell" onClick={() => onOpen(item.view)}>
                    <Glyph item={item} day={day} />
                    <span class="view-name">{item.name}</span>
                    {item.due_count > 0 && <span class="view-due" aria-label={`${item.due_count} due`}>
                        {item.due_count}
                      </span>}
                    {item.open_count > 0 && <span class="view-total">{item.open_count}</span>}
                    <span class="view-chevron">
                      <ChevronRightIcon />
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ))}
      {push && <RemindersRow push={push} />}
    </>
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
