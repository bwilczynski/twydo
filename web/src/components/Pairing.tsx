import { useState } from "preact/hooks";

export function Pairing() {
  const [code, setCode] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(e: Event) {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      const r = await fetch("/pair", {
        method: "POST",
        headers: { "content-type": "application/json" },
        credentials: "same-origin",
        body: JSON.stringify({ code: code.trim() }),
      });
      if (r.ok) {
        location.reload();
        return;
      }
      const body = (await r.json().catch(() => ({}))) as { error?: string };
      setError(body.error ?? "Pairing failed");
    } catch {
      setError("Can't reach Twydo");
    } finally {
      setBusy(false);
    }
  }

  return (
    <main class="center">
      <h1>Pair this phone</h1>
      <p class="hint">Add Twydo to your Home Screen first, then pair inside the app.</p>
      <form onSubmit={submit}>
        <input
          aria-label="Pairing code"
          placeholder="XXXX-XXXX"
          autocapitalize="characters"
          autocomplete="off"
          autocorrect="off"
          spellcheck={false}
          value={code}
          onInput={(e) => setCode((e.target as HTMLInputElement).value)}
        />
        <button type="submit" class="primary" disabled={busy || code.trim() === ""}>
          Pair
        </button>
        {error && (
          <p class="urgent" role="alert">
            {error}
          </p>
        )}
      </form>
    </main>
  );
}
