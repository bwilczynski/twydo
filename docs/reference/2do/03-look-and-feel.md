# 03 — Look and feel

Visual observations below were made from the vendor's promotional screenshots.
Those copyrighted images are excluded from this repository; task text in the diagrams is demo content.

## 1. Window layout

```
┌──────────────┬───────────────────────────────────────────────────┬──────┐
│ ● ● ●   [▯]  │  +                                    🔍▾    [▯]  │ Tags │
│              │                                                   │panel │
│ ⊞ All  10|14 │  Today  Tuesday, 14 July 2026    🕒 │ ↓ Due Date▾ │ ☀ ▾ │(opt.)│
│ 14 Today     │                                                   │      │
│ ☆ Starred  3 │  OVERDUE                                       9  │      │
│ 🕒 Scheduled │  ○ Submit Q2 budget notes   (work)(waiting) 🕒30 m │      │
│ ✓ Done       │                                4 days ago   ≡   │      │
│              │  ↺ Pay electricity bill  (home) 📍 🕒15 m  …  ≡   │      │
│ Personal     │  ○ Call Alex about demo…  …                       │      │
│ ▌Home   2|2  │     ↳ 🗂 Product launch video                      │      │
│ ▌Family 1|2  │                                                   │      │
│ ▌Errands     │  TOMORROW                                      3  │      │
│ Work         │  …                                                │      │
│ ▌Work        │                                          · minimap│      │
│ + ▦       ⟳  │                                                   │      │
└──────────────┴───────────────────────────────────────────────────┴──────┘
```

- **Sidebar** (left, grey-tinted, ~25% width, toggled by the sidebar button or ⌥⌘←):
  - Focus lists at top with outline icons: grid ⊞ **All**, calendar with day number **Today**, ☆ **Starred**, clock **Scheduled**, check-circle **Done** (and tray **Inbox** when on).
  - **List groups** as bold grey headings ("Personal", "Work", "Travel"); hovering shows `+` and a collapse ▾. [obs]
  - **Lists**: name with a **solid colour block on the left edge** (the list colour, about 32 px wide and full row height). Neighbouring lists' blocks touch, so a group reads as a stacked colour bar. Disclosure chevron when the list has projects; projects are indented with a briefcase icon.
  - Selected row: list-coloured background over the full row, white text. Focus lists use a light grey rounded highlight.
  - Counts right-aligned: red overdue/due count `|` grey total.
  - "SMART LISTS" heading; smart lists use a gear ⚙ icon over their colour block.
  - Bottom bar: `+` (new list), calendar toggle ▦, sync ⟳.
- **Toolbar** (window top, no visible separator): sidebar toggle, `+` new task, then right-aligned 🔍 search (with ▾ presets) and the right-panel toggle (tags panel).
- **List header**: list title in **large bold** (~28 px). Today adds the full date in lighter grey. Coloured lists show a small dot in the list colour before the title. The controls on the right are 🕒 show-scheduled toggle (blue filled when active) │ ↓ sort direction · "Due Date ▾" sort field │ ☀ focus filter (yellow when active) ▾.
- **Colour strip**: in multi-list views (All, Today, smart lists), each task row has a **thin vertical bar of its list's colour at the far left edge** of the content area.
- **Minimap**: a column of small dashes/dots at the far right shows the groups; the current one is darker. [obs]
- Large content margins: the list is centred with generous left padding (~120 px) and right padding.

## 2. Group headers

- UPPERCASE, letter-spaced (~0.08 em), semibold, ~15 px.
- **OVERDUE in dark red** (`OverdueColor`), others in dark grey. Muted (light grey) when the group is in the past or empty-ish.
- Task count right-aligned in the same colour; optional total duration in purple after the title ("TODAY 15m").
- **Sections** (named) render as a **larger bold black title with a 1 px rule underneath**, and date groups nest inside them. [obs]

## 3. Task row anatomy

Height about 64 px at 2× (≈32 pt); separator: 1 px light line, inset from the checkbox.

```
[chevron] [checkbox] Title ………………  (tag)(tag)  📍 📞 ✉ 🔗  🕒 30 m   Due text   [priority bars][★]
                     ↳ 🗂 Parent project name (second line, smaller, only outside the project)
```

- **Checkbox**: 22 px thin grey circle. Variants: ↺ circular arrow (repeating), 🕒 clock (scheduled), box with count (project/checklist), small circle (sub-task).
- **Title**: ~17 px regular, near-black (`taskListTaskTitle`); **overdue → dark red**; scheduled → muted blue-grey; long titles **fade out** (gradient truncation, not an ellipsis). [obs]
- **Tags**: pill bubbles, light blue-grey fill (`taskListTagBubbleColor`), dark grey text, fully rounded corners, ~6 px gap.
- **Indicator icons** (outline, blue): 📍 location, 📞/✉ action type, 🔗 link/attachment.
- **Duration**: 🕒 clock outline + "30 m" in **purple** (`DurationColor`).
- **Date column** (right-aligned, fixed width): relative date text. Blue-grey normally (`taskListDueDateColor`), **dark red when overdue**. When a start date exists, two lines: start in **green** (`StartDateColor`) above, due below.
- **Priority**: stacked horizontal bars at the far right, 3 = high (red/orange-red), 2 = medium (orange). A **yellow star** overlaps the bars' top-right when starred.
- **Selection**: full-width rounded rectangle (`taskListTaskSelectionActiveColor`, pale blue), radius ~10 px.

## 4. Inline editor (expanded task)

Inline editor [obs]: the row expands into a **card** with a soft shadow and rounded corners, inset from the list margins.

1. **Above the card**: "Created: Today at 6:57 PM" (grey), and on the right icons: duplicate, share, ⌘ (shortcut help), **red trash**.
2. **Title row**: checkbox + editable title (selected text highlighted); priority/star control at the right.
3. **Property bar** (light blue-grey band):
   - left: **list selector as a slanted tab** filled with the list colour, white text "Family ⌃⌄" (a parallelogram edge on the right).
   - middle: action button (circle, e.g. envelope) · **"Due in 13h 31m"** in blue (or "Starts in 4 days · 90 m" with duration in purple).
   - right: round outline icon buttons: image, audio, location, calendar, repeat ↻, alarm ⏰, duration ⏱ (active ones get a grey circle background).
4. **Tags row**: tag icon + dark-grey pills (white text) + inline tag input.
5. **Dates block** right-aligned: header labels "Start  Time ▶" (green ▶) and "Due date  Time ✓"; values like "Wed, 15 Jul ⊗  9:00 AM ⊗" with clear buttons.
6. **Notes area**: **cream / off-white background** (≈ `#F9F8F0`), "Notes" placeholder, side icons for Markdown (M) and add attachment (+).

## 5. Quick Entry window

Quick entry [obs]: a floating panel (rounded ~16 px, shadow, light grey frame) that holds the **same card as the inline editor**, plus a footer: `Task ⌃⌄` type popup (left), **Cancel** (grey) and **Save** (blue, default) on the right. Date fields show a dropdown of suggestions: rows "Sat, 18 Jul … in 4 days", first row highlighted blue.

## 6. Pickers and popovers

- **Date picker / defer popover**: grey translucent popover, title "Due date of 2 tasks…" + blue **Save**; a white rounded panel with **two months side by side** (S M T W T F S), today in blue text, the selected day in a blue rounded square; a date text field tab and a "Time" checkbox + time field; **4 large grey quick buttons** with ⌥1–⌥4 keycap hints below; a divider, then "Postpone by [ field ] [Days ⌃⌄]".
- **Action picker**: "Pick an Action" grid of 6 coloured rounded buttons with keycap hints: green phone ⌥1, green message ⌥2, blue mail ⌥3, red car ⌥4, blue link ⌥5, blue search ⌥6.
- **Sort menu** and **focus-filter menu**: standard popup menus with checkmarks and separators.
- **Quick Open**: a wide rounded search field over the content; results grouped "Matched 1 Task" / "Matched 2 Lists"; the selected row has a blue fill with the parent list as a subtitle and a coloured dot.
- **Search field mode**: the toolbar becomes a long rounded search field with a preset icon, query text, a clear ⊗, and a **token chip** for the date preset ("Coming 3 days ✕"); the header shows a red 🔍 + smart list name.

## 7. Colour palette (exact) [obs]

| Token | Light | Dark | Use |
|---|---|---|---|
| AccentColor | `#4B96ED` | `#5CA4F6` | buttons, active toggles, links |
| OverdueColor | `#B03429` | `#D94032` | OVERDUE header, overdue dates |
| taskListTaskTitleOverdue | `#8A1A10` | `#E65444` | overdue task titles |
| StartDateColor | `#2C7214` | `#349E19` | start-date text, ▶ |
| DurationColor | `#663591` | `#457BB1` | duration text/icon |
| taskListDueDateColor | `#4B6082` | `#81A6E2` | normal due-date text |
| taskListTaskTitle | `#141414` | `#F2F2F2` | task title |
| taskListTaskTitleScheduledColor | `#3A3D58` (α .95) | `#969EDE` (α .95) | scheduled task titles |
| taskListTaskTitleDurationDueSoonColor | `#333592` | `#7C7EE7` | "due soon" emphasis |
| taskListTagBubbleColor | `#D7E1FF` | `#656567` | tag pill fill |
| taskListTaskSelectionActiveColor | `#E0EAF6` | `#262F3C` | selected row |
| taskListTaskSelectionInactiveColor | `#D9F7F8` | `#2D3236` (α .94) | selected row, window inactive |
| taskListSearchHighlightBackgroundColor | `#FCFD63` | `#0059D1` | search match highlight |
| CheckboxColor | `#1E1E1F` (α .40) | `#FFA0A0` | checkbox stroke |
| BaseColorBand | `#E1E4E8` (α .50) | `#24262A` | background colour bands |
| listPanelTextOverDarkBG | `#1F1F1F` (α .90) | `#DBDBDB` (α .91) | sidebar text |
| listPanelGroupTextOverDarkBG | `#666666` (α .95) | `#8C8C8C` (α .90) | sidebar group headings |

Additional (approximate, sampled from screenshots) [inf]:
- Sidebar background ≈ `#D9DEE3` (light) — a cool grey.
- Editor property band ≈ `#E5E9F3`; notes area ≈ `#F9F8F0`.
- Priority bars: high ≈ `#E5533D`, medium ≈ `#F0A030`; star ≈ `#F5C518`.
- **Preset list colour palette** (hex table shipped in the app, in this order) [obs]:
  `#3289D2` blue · `#379417` green · `#6946CD` purple · `#75715B` olive · `#A32124` dark red · `#FF8B00` orange · `#55B456` light green · `#FFC0BD` pink · `#B08DAA` mauve · `#EEDC1B` yellow · `#4D4B4B` charcoal.
  Extended set: `#9269D3 #3A8F8B #C96F5B #5967A8 #7D873E #B85F7A #627A9D #B58B57 #6F9475 #8B5C8E #CA66B5 #C63E43 #DC8300 #08B1C8 #6C6657`.
- List colours are stored as `r,g,b` floats. Support arbitrary RGB colours alongside presets.

## 8. Typography

- macOS system font (**SF Pro**). On Linux use **Inter** (closest metrics) or the Omarchy theme font for chrome, keeping these sizes (at 1×): list title 28 semibold/bold · task title 17 regular · group header 15 semibold uppercase tracked · meta text (dates, tags, durations) 15 regular · sidebar items 17 regular · sidebar group headings 15 semibold grey.
- Font size is user-adjustable (View › Font Size, 11 steps). [obs]
- Numbers in counts and dates: tabular figures.

## 9. Icons

2Do's icons are **outline style, ~1.5 px stroke**, matching SF Symbols. [obs] On Linux use **Lucide** or **Phosphor (regular)**, which match closely: `layout-grid` (All), `calendar` with day number (Today), `star`, `clock`, `circle-check`, `inbox`, `briefcase` (project), `list-checks` (checklist), `map-pin`, `phone`, `mail`, `link`, `timer`, `repeat`/`rotate-ccw`, `alarm-clock`, `tag`, `sun` (focus), `search`, `panel-left`, `panel-right`, `plus`.

## 10. Motion and interaction details

- Rows expand into the editor in place (smooth height animation); other rows shift down.
- Completing a task: the checkbox fills, then the row fades or slides out after a short delay. [inf]
- Hover reveals `+` and the collapse control on sidebar group headings.
- Drag and drop: tasks onto sidebar lists (move), onto section titles (move to section), onto the Scheduled list (set start date), onto calendar days (defer).
- Double-click on empty space creates a new task ("Click on [+] or 'double click' to create a new task"). [obs]

## 11. Omarchy notes

- Omarchy is Hyprland-based and themed. Keep 2Do's palette as the "2Do theme", and optionally map tokens to the active Omarchy theme colours.
- Hyprland draws no title bar, so drop the traffic lights. Keep the same toolbar row with sidebar toggle, `+`, search and right-panel toggle.
- Light and dark both matter: every token above has a dark value.
