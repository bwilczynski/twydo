# TUI Layout, Theme, and Focus Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan in this session. Steps use checkbox syntax for tracking.

**Goal:** Implement the approved responsive sidebar, terminal styling, and unambiguous pane focus.
**Architecture:** Keep Ratatui rendering in ui modules. Share semantic styles in ui/theme.rs. Let App track Details visibility so navigation and resize never focus a hidden pane.
**Tech Stack:** Existing Rust/Ratatui workspace; no new dependencies.
**Spec:** docs/superpowers/specs/2026-11-01-tui-layout-theme-focus-design.md

## Global Constraints

- All chrome uses terminal defaults or the 16 ANSI colors. RGB is reserved for list bars.
- Synthetic fixtures only. No production data, terminal configuration changes, or sync operations.
- Preserve h/l and Tab pane order, task selection, overdue red, and list colors.
- Hold commits while the working-tree diff is under review.

## Review Focus

- Narrow or resized windows: keyboard focus must stay on a visible pane.
- Wide Unicode list names: additional sidebar space should reveal names without losing counts.
- Overdue selected tasks: preserve red text and RGB bars without reverse video.
- Status prompts: focus labels must not overwrite search, commands, or unreadable-file warnings.
- Small dimensions and deeply nested rows: marker width must not break safe truncation.

### Task 1: Responsive sidebar

**Files:** ui/mod.rs and tests/ui_render.rs under crates/tdo/src and crates/tdo respectively.
**Interface:** existing draw(&mut Frame, &App).
- [x] Add render tests for sidebar growth and longer Unicode names with aligned counts.
- [x] Run cargo test -p tdo --test ui_render and confirm the new tests fail.
- [x] Use width/5 clamped to 24..40 from 60 columns; retain width/3 below 60. Make test task-pane lookup discover pane boundaries.
- [x] Run rendering tests.

### Task 2: Theme and focus rendering

**Files:** crates/tdo/src/ui/{theme,mod,tasks,sidebar,status,details}.rs; crates/tdo/tests/ui_render.rs.
**Interface:** private semantic style functions; unchanged public draw interface.
- [x] Add regression tests through real l/h/Tab key sequences, active/inactive task styles, selected overdue/list colors, sidebar selection, status prompts, and ANSI-only chrome.
- [x] Run rendering tests and confirm failures.
- [x] Add shared style roles, blue active borders/titles and mode badge, palette-based selection background, and a muted one-cell inactive task marker. Keep list bars and overdue colors intact.
- [x] Add focused-pane status text when it fits without truncating prompts.
- [x] Run rendering tests.

### Task 3: Hidden Details focus and validation

**Files:** crates/tdo/src/{app,term}.rs, crates/tdo/tests/{app_keys,ui_render}.rs; docs/design/tui-layout.md.
**Interface:** App::resize(width: u16, height: u16); shared Details width threshold of 110.
- [x] Add tests for narrow Tab/l navigation, wide-to-narrow resizing, and preserved task selection.
- [x] Run cargo test -p tdo --test app_keys and confirm failures.
- [x] Update initial terminal sizing and Resize events to normalize focus, set page height, and skip hidden Details during navigation.
- [x] Document final layout and styling.
- [x] Run cargo fmt --all -- --check, cargo test --workspace, cargo clippy --workspace --all-targets -- -D warnings, and git diff --check.
- [x] Review the whole diff and visually inspect a synthetic-data TUI; leave changes available for user review.
