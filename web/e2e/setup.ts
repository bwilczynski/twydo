import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../..");
const tdo = process.env.TDO_BIN ?? join(repo, "target/debug/tdo");

/** Environment of every `tdo` the suite runs: its own config, state and socket, never the user's. */
export function tdoEnv(): NodeJS.ProcessEnv {
  const env = { ...process.env };
  delete env.TWYDO_SOCKET;
  return {
    ...env,
    TWYDO_SOCKET: process.env.E2E_SOCKET,
    XDG_CONFIG_HOME: process.env.E2E_CONFIG,
    XDG_STATE_HOME: process.env.E2E_XDG_STATE,
    XDG_RUNTIME_DIR: process.env.E2E_RUNTIME,
    HOME: process.env.E2E_ROOT,
    TWYDO_NOTIFIER: "off",
  };
}

export function tdoRun(args: string[]): string {
  return execFileSync(tdo, args, { env: tdoEnv(), encoding: "utf8" });
}

/** Starts `tdo daemon` in the background; callers wait for it with {@link waitForDaemon}. */
export function startDaemon(): ChildProcess {
  const child = spawn(tdo, ["daemon"], { env: tdoEnv(), stdio: "ignore", detached: true });
  child.unref();
  return child;
}

export async function waitForDaemon(): Promise<void> {
  const deadline = Date.now() + 20_000;
  for (;;) {
    try {
      tdoRun(["daemon", "ping"]);
      return;
    } catch {
      if (Date.now() > deadline) throw new Error("the test daemon never answered");
      await new Promise((r) => setTimeout(r, 100));
    }
  }
}

export function stopDaemon(): void {
  try {
    tdoRun(["daemon", "stop"]);
  } catch {
    // already stopped
  }
}

/** A single-use pairing code for a new device. */
export function pairingCode(name: string): string {
  const out = tdoRun(["gateway", "--state-dir", process.env.E2E_GATEWAY_STATE!, "pair", name]);
  const m = /Pairing code for .*?: (\S+) \(valid/.exec(out);
  if (!m) throw new Error(`no pairing code in: ${out}`);
  return m[1];
}

const ics = (uid: string, summary: string, due?: string) =>
  [
    "BEGIN:VCALENDAR",
    "VERSION:2.0",
    "PRODID:-//synthetic//EN",
    "BEGIN:VTODO",
    `UID:${uid}`,
    `SUMMARY:${summary}`,
    ...(due ? [`DUE;VALUE=DATE:${due}`] : []),
    "END:VTODO",
    "END:VCALENDAR",
    "",
  ].join("\r\n");

function today(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}`;
}

async function freePort(): Promise<number> {
  return new Promise((ok, fail) => {
    const s = createServer();
    s.listen(0, "127.0.0.1", () => {
      const port = (s.address() as { port: number }).port;
      s.close(() => ok(port));
    });
    s.on("error", fail);
  });
}

async function waitForPort(port: number): Promise<void> {
  const deadline = Date.now() + 20_000;
  for (;;) {
    try {
      const r = await fetch(`http://127.0.0.1:${port}/`);
      if (r.status < 500) return;
    } catch {
      // not listening yet
    }
    if (Date.now() > deadline) throw new Error("the gateway never accepted connections");
    await new Promise((r) => setTimeout(r, 100));
  }
}

export default async function globalSetup(): Promise<() => Promise<void>> {
  const root = mkdtempSync(join(tmpdir(), "twydo-e2e-"));
  // Unix socket paths are short-limited: the socket lives directly under /tmp, mode 0700.
  const sockDir = mkdtempSync("/tmp/td");
  chmodSync(sockDir, 0o700);
  const vdir = join(root, "vdir");
  mkdirSync(join(vdir, "inbox"), { recursive: true });
  mkdirSync(join(vdir, "work"), { recursive: true });
  mkdirSync(join(root, "config/twydo"), { recursive: true });
  mkdirSync(join(sockDir, "run"), { recursive: true, mode: 0o700 });

  const t = today();
  for (let i = 1; i <= 3; i++) {
    writeFileSync(join(vdir, "inbox", `i${i}.ics`), ics(`i${i}`, `Synthetic task ${i}`));
  }
  writeFileSync(join(vdir, "work", "w1.ics"), ics("w1", "Synthetic due today", t));
  writeFileSync(join(vdir, "work", "w2.ics"), ics("w2", "Synthetic also due today", t));
  writeFileSync(
    join(root, "config/twydo/config.toml"),
    `vdir = "${vdir}"\nstate_dir = "${join(root, "state")}"\n[writes]\nmode = "test"\n`,
  );

  const port = await freePort();
  Object.assign(process.env, {
    E2E_ROOT: root,
    E2E_SOCKET: join(sockDir, "tdo.sock"),
    E2E_CONFIG: join(root, "config"),
    E2E_XDG_STATE: join(root, "xdg-state"),
    E2E_RUNTIME: join(sockDir, "run"),
    E2E_GATEWAY_STATE: join(root, "gateway"),
    E2E_BASE_URL: `http://127.0.0.1:${port}`,
  });

  let gateway: ChildProcess | undefined;
  const teardown = async () => {
    gateway?.kill();
    stopDaemon();
    rmSync(root, { recursive: true, force: true });
    rmSync(sockDir, { recursive: true, force: true });
  };
  try {
    startDaemon();
    await waitForDaemon();
    gateway = spawn(
      tdo,
      [
        "gateway",
        "--state-dir",
        process.env.E2E_GATEWAY_STATE!,
        "serve",
        "--insecure",
        "--origin",
        `http://127.0.0.1:${port}`,
        "--listen",
        `127.0.0.1:${port}`,
        "--assets",
        join(repo, "web/dist"),
      ],
      { env: tdoEnv(), stdio: "ignore" },
    );
    await waitForPort(port);
  } catch (e) {
    await teardown();
    throw e;
  }
  return teardown;
}
