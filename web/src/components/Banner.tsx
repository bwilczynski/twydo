import { useEffect } from "preact/hooks";

export interface BannerState {
  id: number;
  text: string;
  undo: boolean;
  /** A second button, e.g. to reopen a refused capture. */
  action?: { label: string; run: () => void };
}

export const BANNER_MS = 4000;
// A banner offering an action stays longer, so it can be reached.
export const BANNER_ACTION_MS = 10000;

export function Banner({
  banner,
  undoDisabled,
  onUndo,
  onDismiss,
}: {
  banner: BannerState;
  undoDisabled: boolean;
  onUndo: () => void;
  onDismiss: () => void;
}) {
  useEffect(() => {
    const t = setTimeout(onDismiss, banner.action ? BANNER_ACTION_MS : BANNER_MS);
    return () => clearTimeout(t);
  }, [banner.id]);
  return (
    <div class="banner" role="status">
      <span>{banner.text}</span>
      {banner.undo && (
        <button type="button" disabled={undoDisabled} onClick={onUndo}>
          Undo
        </button>
      )}
      {banner.action && (
        <button type="button" onClick={banner.action.run}>
          {banner.action.label}
        </button>
      )}
    </div>
  );
}
