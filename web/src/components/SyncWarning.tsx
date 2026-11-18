import type { SyncSummary } from "../gen/protocol";

export function SyncWarning({ sync }: { sync: SyncSummary }) {
  return (
    <div class="sync-warning" role="alert">
      <strong>{sync.headline}</strong>
      {sync.detail && <p>{sync.detail}</p>}
    </div>
  );
}
