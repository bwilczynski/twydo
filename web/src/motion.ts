// What changed between two fetches of the same view, so the list can animate rows in and out
// instead of jumping. Pure; the list component owns the timing.
import type { SectionKey, ViewRow, ViewSection } from "./gen/protocol";

/** A row the new view no longer has, kept on screen while it animates out. */
export interface Leaving {
  row: ViewRow;
  group: SectionKey;
  /** `sectionId` of its old section. */
  section: string;
  label: string | null;
  /** The row above it in its old section, or null when it was first. */
  after: string | null;
  /** The section above its old section, or null when that was first. */
  afterSection: string | null;
  /** Shown ticked while it leaves, as it was when the view changed. */
  done: boolean;
}

export interface ShownSection {
  /** Unique among the shown sections; see `sectionId`. */
  id: string;
  group: SectionKey;
  label: string | null;
  rows: { row: ViewRow; leaving?: Leaving }[];
}

/** Tells sections apart: `group` alone repeats for each completed day, so a day is added. */
export const sectionId = (s: ViewSection): string => (s.date ? `${s.group}:${s.date}` : s.group);

const sectionsByUid = (secs: ViewSection[]) =>
  new Map(secs.flatMap((s) => s.rows.map((r) => [r.uid, sectionId(s)] as const)));

/** Rows of `old` that `next` does not have anywhere, in their old order. */
export function departures(old: ViewSection[], next: ViewSection[], done: Set<string>): Leaving[] {
  const live = sectionsByUid(next);
  const out: Leaving[] = [];
  old.forEach((s, i) =>
    s.rows.forEach((row, j) => {
      if (live.has(row.uid)) return;
      out.push({
        row,
        group: s.group,
        section: sectionId(s),
        label: s.label ?? null,
        after: j > 0 ? s.rows[j - 1]!.uid : null,
        afterSection: i > 0 ? sectionId(old[i - 1]!) : null,
        done: done.has(row.uid) || row.completed,
      });
    }),
  );
  return out;
}

/** Rows of `next` that are new, or in a different section than in `old`. None on a first view. */
export function arrivals(old: ViewSection[] | undefined, next: ViewSection[]): Set<string> {
  if (!old) return new Set();
  const was = sectionsByUid(old);
  return new Set(next.flatMap((s) => s.rows.filter((r) => was.get(r.uid) !== sectionId(s)).map((r) => r.uid)));
}

/** `next`'s sections with the leaving rows put back where they were. */
export function display(next: ViewSection[], leaving: Leaving[]): ShownSection[] {
  const out: ShownSection[] = next
    .filter((s) => s.rows.length > 0)
    .map((s) => ({ id: sectionId(s), group: s.group, label: s.label ?? null, rows: s.rows.map((row) => ({ row })) }));
  for (const l of leaving) {
    let sec = out.find((s) => s.id === l.section);
    if (!sec) {
      sec = { id: l.section, group: l.group, label: l.label, rows: [] };
      const at = l.afterSection === null ? -1 : out.findIndex((s) => s.id === l.afterSection);
      out.splice(at + 1, 0, sec);
    }
    const at = l.after === null ? -1 : sec.rows.findIndex((x) => x.row.uid === l.after);
    sec.rows.splice(at + 1, 0, { row: l.row, leaving: l });
  }
  return out;
}
