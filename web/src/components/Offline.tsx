export function Offline({ updated, onRetry }: { updated?: string; onRetry: () => void }) {
  return (
    <div class="offline">
      <p class="offline-title">Can't reach Twydo</p>
      {updated && <p class="hint">last updated {updated}</p>}
      <button type="button" class="primary" onClick={onRetry}>
        Retry
      </button>
    </div>
  );
}
