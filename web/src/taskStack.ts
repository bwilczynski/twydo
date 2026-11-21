import { useEffect, useRef, useState } from "preact/hooks";
import { navigate, type Direction } from "./navigate";
import { parseTaskHash, taskHash } from "./route";

/**
 * The open task screens, newest last, kept in step with the browser history: opening pushes an
 * entry, the browser's back pops one. `base` counts entries this page did not push (a reload
 * on a task URL), which back must replace rather than leave. Pushes and pops slide (`navigate`);
 * leaving every screen for a tab does not, as tab switches never do.
 */
export function useTaskStack() {
  const [stack, setStack] = useState<string[]>(() => {
    const uid = parseTaskHash(location.hash);
    return uid ? [uid] : [];
  });
  const ref = useRef(stack);
  const base = useRef(stack.length);
  // Set while history.go() travels to a reloaded task entry that is about to be replaced.
  const clearing = useRef(false);

  // The ref moves at once; only the render waits for the slide to take its snapshot, and then
  // shows the newest stack, in case an instant change came in meanwhile.
  const set = (next: string[], dir?: Direction) => {
    ref.current = next;
    if (next.length === 0) base.current = 0;
    if (dir) navigate(dir, () => setStack(ref.current));
    else setStack(next);
  };
  const pushed = () => Math.max(0, ref.current.length - base.current);
  const toRoot = () => history.replaceState(null, "", location.pathname + location.search);

  useEffect(() => {
    const onPop = (e: PopStateEvent) => {
      if (clearing.current) {
        clearing.current = false;
        toRoot();
        set([]);
        return;
      }
      const saved = e.state?.stack;
      const uid = parseTaskHash(location.hash);
      const next: string[] = Array.isArray(saved) ? saved : uid ? [uid] : [];
      const n = ref.current.length;
      // The popstate of a pop() that already moved: rendering now would land before its slide
      // takes the outgoing snapshot, which would then show the destination twice.
      if (next.length === n && next.every((u, i) => u === ref.current[i])) return;
      // Safari animates its own swipe back.
      const dir = e.hasUAVisualTransition || next.length === n ? undefined : next.length > n ? "forward" : "back";
      set(next, dir);
    };
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  return {
    stack,
    /** The uids as of now, even before a render. */
    current: () => ref.current,
    open(uid: string) {
      const next = [...ref.current, uid];
      history.pushState({ stack: next }, "", taskHash(uid));
      set(next, "forward");
    },
    /** One screen back: the previous entry, or the tab root for the first one. */
    pop() {
      const next = ref.current.slice(0, -1);
      if (pushed() > 0) history.back();
      else toRoot();
      set(next, "back");
    },
    /** Every screen, back to the tab root. */
    clear() {
      const n = pushed();
      if (n > 0) {
        clearing.current = base.current > 0;
        history.go(-n);
      } else {
        toRoot();
      }
      set([]);
    },
  };
}
