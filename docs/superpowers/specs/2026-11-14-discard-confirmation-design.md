# Discard confirmation for the edit card (#99)

## Intent

`Esc` on the task edit card closes it at once and loses everything typed. When the card holds
unsaved changes, `Esc` should ask before discarding them.

## Goals and non-goals

- Goal: a dirty task edit card (existing or new task) asks `y`/`n` before discarding on `Esc`.
- Goal: a clean card still closes on one `Esc`.
- Non-goal: the list card, the one-line new row, and quitting with `Ctrl-c`.
- Anti-goal: an `Esc` that writes anything, or a "keep editing" that loses field values.

## Approaches

1. **Prompt in the card's footer (chosen).** The card's bottom-border hint becomes the question;
   the card stays drawn in place. Keeps the question next to what it is about and matches the
   card's in-place layout.
2. Overlay through the app's existing `Confirmation` enum. Reuses the overlay, but hides the
   card behind it and mixes a pure form concern into the app-level confirmation set.
3. Make `Esc` always close and offer undo. Nothing is written, so there is nothing to undo.

## Design

- **Dirty:** a card is dirty when any field (including List) differs from the values it opened
  with. A new task's card is also dirty while its title is non-empty, since the title was typed
  in the new row before the card opened.
- `EditForm` gains a `discarding: bool`. `Esc` on a dirty card sets it and returns
  `FormAction::None`; on a clean card it returns `FormAction::Cancel` as today.
- While `discarding`: `y` returns `Cancel` (the card closes, nothing written); `n` or `Esc`
  clears it with every field kept; other keys are ignored. Paste is ignored too.
- The card's footer reads `Discard changes? y discard · n/Esc keep editing` while discarding,
  else the usual hints. The overlay fallback shows the same.
- `docs/tui.md` (edit card section) describes the prompt.

## Left to the builder

Exact styling of the footer question; test placement follows existing `form.rs` and card UI
tests.
