// The task screen's place in the URL hash.
const PREFIX = "#/task/";

export const TODAY_HASH = "#/today";

export const taskHash = (uid: string): string => PREFIX + encodeURIComponent(uid);

export function parseTaskHash(hash: string): string | undefined {
  if (!hash.startsWith(PREFIX) || hash.length === PREFIX.length) return undefined;
  try {
    return decodeURIComponent(hash.slice(PREFIX.length));
  } catch {
    return undefined;
  }
}
