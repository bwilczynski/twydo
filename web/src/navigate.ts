// Pushing and popping screens the iOS way: the new screen slides in from the right, and back
// slides the top one away. Built on view transitions; where they are missing, or the phone asks
// for reduced motion, the screen changes at once as before.

export type Direction = "forward" | "back";

// The snapshot of the new screen waits for the render the update causes; App reports each commit.
let waiting: (() => void)[] = [];
// An update that renders nothing must not hold the screen frozen.
const LIMIT_MS = 150;
// Tells the newest transition from one it cut short, whose end must not clear its direction.
let seq = 0;

/** Called after every App render commits: the new screen is in the DOM. */
export function committed() {
  const w = waiting;
  waiting = [];
  w.forEach((resolve) => resolve());
}

type Start = (update: () => Promise<void>) => { finished: Promise<void> };

/** Run `update`, a state change that pushes or pops a screen, animated in `dir`. */
export function navigate(dir: Direction, update: () => void) {
  const start = (document as unknown as { startViewTransition?: Start }).startViewTransition;
  if (!start || matchMedia("(prefers-reduced-motion: reduce)").matches) {
    update();
    return;
  }
  const root = document.documentElement;
  const id = ++seq;
  root.dataset.nav = dir;
  const t = start.call(
    document,
    () =>
      new Promise<void>((resolve) => {
        waiting.push(resolve);
        setTimeout(resolve, LIMIT_MS);
        update();
      }),
  );
  t.finished.finally(() => {
    if (id === seq) delete root.dataset.nav;
  });
}
