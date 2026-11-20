import { type ChildProcess, execFileSync, spawn } from "node:child_process";
import {
  chmodSync,
  closeSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../..");
const tdo = process.env.TDO_BIN ?? join(repo, "target/debug/tdo");

/**
 * Environment of every `tdo` the suite runs: its own config, state, data, cache, runtime and
 * socket, never the user's. It fails closed: without the isolated paths it throws.
 */
export function tdoEnv(): NodeJS.ProcessEnv {
  const need = ["E2E_SOCKET", "E2E_CONFIG", "E2E_XDG_STATE", "E2E_RUNTIME", "E2E_ROOT"] as const;
  for (const k of need) {
    if (!process.env[k]) throw new Error(`e2e isolation: ${k} is not set; refusing to run tdo`);
  }
  const env = { ...process.env };
  delete env.TWYDO_SOCKET;
  return {
    ...env,
    TWYDO_SOCKET: process.env.E2E_SOCKET,
    XDG_CONFIG_HOME: process.env.E2E_CONFIG,
    XDG_STATE_HOME: process.env.E2E_XDG_STATE,
    XDG_DATA_HOME: join(process.env.E2E_ROOT!, "xdg-data"),
    XDG_CACHE_HOME: join(process.env.E2E_ROOT!, "xdg-cache"),
    XDG_RUNTIME_DIR: process.env.E2E_RUNTIME,
    HOME: process.env.E2E_ROOT,
    TWYDO_NOTIFIER: "off",
  };
}

export function tdoRun(args: string[]): string {
  return execFileSync(tdo, args, { env: tdoEnv(), encoding: "utf8" });
}

const logPath = (name: string) => join(process.env.E2E_ROOT!, `${name}.log`);

/** Starts `tdo daemon` in the background, logging to the temp dir; its pid is recorded. */
export function startDaemon(): ChildProcess {
  const log = openSync(logPath("daemon"), "a");
  const child = spawn(tdo, ["daemon"], {
    env: tdoEnv(),
    stdio: ["ignore", log, log],
    detached: true,
  });
  closeSync(log);
  child.unref();
  if (child.pid) writeFileSync(join(process.env.E2E_ROOT!, "daemon.pid"), String(child.pid));
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

/** Stops the daemon; kills it by recorded pid when `daemon stop` fails or times out. */
export function stopDaemon(): void {
  try {
    execFileSync(tdo, ["daemon", "stop"], { env: tdoEnv(), timeout: 15_000, stdio: "ignore" });
    return;
  } catch {
    // fall through to the pid
  }
  try {
    const pidFile = join(process.env.E2E_ROOT!, "daemon.pid");
    if (existsSync(pidFile)) process.kill(Number(readFileSync(pidFile, "utf8")), "SIGKILL");
  } catch {
    // already gone
  }
}

function dumpLogs(): void {
  for (const n of ["daemon", "gateway"]) {
    try {
      console.error(`--- ${n}.log ---\n${readFileSync(logPath(n), "utf8")}`);
    } catch {
      // no log
    }
  }
}

/** A single-use pairing code for a new device. */
export function pairingCode(name: string): string {
  const out = tdoRun(["gateway", "--state-dir", process.env.E2E_GATEWAY_STATE!, "pair", name]);
  const m = /Pairing code for .*?: (\S+) \(valid/.exec(out);
  if (!m) throw new Error(`no pairing code in: ${out}`);
  return m[1];
}

const ics = (uid: string, summary: string, due?: string, extra = "") =>
  [
    "BEGIN:VCALENDAR",
    "VERSION:2.0",
    "PRODID:-//synthetic//EN",
    "BEGIN:VTODO",
    `UID:${uid}`,
    `SUMMARY:${summary}`,
    ...(due ? [`DUE;VALUE=DATE:${due}`] : []),
    ...(extra ? extra.split("\n") : []),
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
  mkdirSync(join(vdir, "errands"), { recursive: true });
  mkdirSync(join(root, "config/twydo"), { recursive: true });
  mkdirSync(join(sockDir, "run"), { recursive: true, mode: 0o700 });

  const t = today();
  for (let i = 1; i <= 3; i++) {
    writeFileSync(join(vdir, "inbox", `i${i}.ics`), ics(`i${i}`, `Synthetic task ${i}`));
  }
  writeFileSync(
    join(vdir, "inbox", "chk1.ics"),
    ics("chk1", "Synthetic packing", undefined, "X-2DOAPP-METADATA:@tasktype:checklist"),
  );
  for (const [uid, title] of [
    ["chk1a", "Synthetic socks"],
    ["chk1b", "Synthetic charger"],
  ]) {
    writeFileSync(
      join(vdir, "inbox", `${uid}.ics`),
      ics(uid, title, undefined, "RELATED-TO;RELTYPE=PARENT:chk1"),
    );
  }
  // Seeds the editing specs change, one set per colour scheme: the daemon is shared by both.
  for (const tag of ["light", "dark"]) {
    writeFileSync(
      join(vdir, "inbox", `e-${tag}.ics`),
      ics(
        `e-${tag}`,
        `Synthetic edit ${tag}`,
        undefined,
        'X-2DOAPP-METADATA:<2Do Meta>{"DisplayOrder":1}</2Do Meta>',
      ),
    );
    writeFileSync(join(vdir, "inbox", `d-${tag}.ics`), ics(`d-${tag}`, `Synthetic delete ${tag}`));
    writeFileSync(
      join(vdir, "inbox", `t-${tag}.ics`),
      ics(`t-${tag}`, `Synthetic trip ${tag}`, undefined, "X-2DOAPP-METADATA:@tasktype:checklist"),
    );
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
    if (gateway && gateway.exitCode === null) {
      const exited = new Promise<void>((r) => gateway!.once("exit", () => r()));
      gateway.kill();
      await Promise.race([exited, new Promise((r) => setTimeout(r, 5_000))]);
      if (gateway.exitCode === null) gateway.kill("SIGKILL");
    }
    stopDaemon();
    rmSync(root, { recursive: true, force: true });
    rmSync(sockDir, { recursive: true, force: true });
  };
  try {
    startDaemon();
    await waitForDaemon();
    const gwLog = openSync(logPath("gateway"), "a");
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
      { env: tdoEnv(), stdio: ["ignore", gwLog, gwLog] },
    );
    closeSync(gwLog);
    await waitForPort(port);
  } catch (e) {
    dumpLogs();
    await teardown();
    throw e;
  }
  return teardown;
}
