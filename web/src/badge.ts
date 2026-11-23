/** The Badging API as far as the app uses it; absent on browsers without it. */
export type BadgeNav = {
  setAppBadge?(n: number): Promise<void>;
  clearAppBadge?(): Promise<void>;
};

/**
 * Shows `n` on the Home Screen icon; 0 removes the badge, `undefined` leaves it. Never throws
 * and never leaves a rejected promise: the badge is a nicety.
 */
export function setBadge(n: number | undefined, nav: BadgeNav = navigator as BadgeNav): void {
  if (n === undefined) return;
  try {
    const p = n === 0 ? nav.clearAppBadge?.() : nav.setAppBadge?.(n);
    p?.catch(() => {});
  } catch {
    // unsupported here
  }
}
