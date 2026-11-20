import { useEffect, useRef, useState } from "preact/hooks";
import { parseTaskHash, taskHash } from "./route";

/**
 * The open task screens, newest last, kept in step with the browser history: opening pushes an
 * entry, the browser's back pops one. `base` counts entries this page did not push (a reload
 * on a task URL), which back must replace rather than leave.
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

  const set = (next: string[]) => {
    ref.current = next;
    if (next.length === 0) base.current = 0;
    setStack(next);
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
      set(Array.isArray(saved) ? saved : uid ? [uid] : []);
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
      set(next);
    },
    /** One screen back: the previous entry, or the tab root for the first one. */
    pop() {
      const next = ref.current.slice(0, -1);
      if (pushed() > 0) history.back();
      else toRoot();
      set(next);
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
