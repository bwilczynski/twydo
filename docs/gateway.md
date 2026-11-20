# The phone gateway

`tdo gateway` serves a small web app for your phone and talks to the daemon for it. You add
the app to the home screen and capture, complete, reopen and undo tasks, and open a task's
screen, from anywhere. It is a client of the daemon like the TUI: it never reads your configuration or your vdir, and the daemon
never opens a network port. The design is in
[the MVP spec](superpowers/specs/2026-11-17-phone-companion-mvp-design.md).

## Security model

- The gateway listens on `127.0.0.1:8787` by default. Put a TLS reverse proxy in front of it
  and let it pass WebSocket upgrades on `/ws`. Do not expose the port directly.
- `--origin` (or `TWYDO_GATEWAY_ORIGIN`) is the public `https://` origin the phone uses. The
  gateway refuses requests from any other origin. It is required, and must be only
  `scheme://host[:port]` (no path; a trailing `/` and upper case are tolerated). `http://` is
  refused unless `--insecure` is given.
- `--insecure` serves plain HTTP on a trusted network: no origin is required and the device
  cookie is not marked `Secure`. Use it only on a network you trust.
- A device pairs once with a code (valid 10 minutes, single use, five wrong attempts void it).
  It then holds a long-lived `HttpOnly` cookie. The gateway stores only hashes of codes and
  keys, in files only you can read.
- Wrong pairing attempts are charged to every pending code. Anyone who can reach `/pair` can
  therefore void a pending code; make a new one. Pair on a trusted network, or quickly.
- A revoked device loses access within 5 seconds, and its open connection is closed.
- Logs name the device and the method, never a task title.

## With Docker

The image `ghcr.io/bwilczynski/twydo` holds `tdo` and vdirsyncer and runs the daemon and the
gateway together. Everything it keeps is under the volume `/data`.

1. Copy `contrib/docker/docker-compose.yaml` and `contrib/docker/env.sample` to a folder. Rename
   the sample to `.env` and set your origin.
2. Put the CalDAV password in `caldav_password` next to them. The container runs as uid 1000,
   so that user must be able to read it (`chown 1000 caldav_password; chmod 400 caldav_password`).
3. Write `data/config/twydo/config.toml` (below). The container runs as uid 1000, so
   `data` must be writable by it.
4. `docker compose up -d`.

The first release publishes the package private: the repository owner makes the GHCR package
public once (package settings, change visibility) before others can pull it.

The compose file binds the port to `127.0.0.1`; your proxy forwards to it.

## Without Docker

Install `tdo` and vdirsyncer ([install.md](install.md)), write the configuration as for the
desktop, and run:

```sh
tdo gateway serve --origin https://tasks.example.test
```

The gateway needs a daemon; any `tdo` command starts one, or install it as a service
(`tdo daemon install`). `contrib/systemd/tdo-gateway.service` is a reference unit for the
gateway itself. Set `TWYDO_GATEWAY_LISTEN` or `--listen` to change the address, and
`TWYDO_GATEWAY_STATE` or `--state-dir` to move its state (default
`$XDG_STATE_HOME/twydo/gateway`).

## The server's configuration

The server has its own `config.toml`, in the same format as the desktop's
([config.md](config.md)). `[writes]` has the same mode and matrix, so the phone gets exactly the
operations the desktop would. In the image the path is `/data/config/twydo/config.toml`:

```toml
vdir = "/data/vdir"

[sync]
url = "https://dav.example.test/dav/"
username = "tasks"
password_command = ["cat", "/run/secrets/caldav_password"]
```

Do not run a second sync from another machine on the same vdir; the server's daemon is the
one that syncs with CalDAV.

## Pairing

Pair from inside the installed home-screen app, not from the browser tab: the home-screen app
keeps its own cookies, so a device paired in Safari is not paired in the app.

```sh
docker compose exec twydo tdo gateway pair phone
# without Docker: tdo gateway pair phone
```

Open the app from the home screen and enter the code. `--role display` pairs a read-only
device.

```sh
tdo gateway devices          # name, role, paired at, last seen
tdo gateway revoke phone
```

## Loading and refreshing

The first time a tab opens, the phone shows a spinner until its view arrives. After that it
keeps the last view of each tab in memory, never on the device's storage: switching tabs or
returning to the app shows it at once, and when the fresh view arrives, rows that are new slide
in and rows that are gone (a completed task, for example) slide out. Only a lost connection
dims the list, under "Can't reach Twydo". An empty list shows its icon, "Nothing here" and, when
a task can be added, a hint to tap **+**.

## Views

The app has three tabs: Inbox, Today and Views. Views lists what the desktop sidebar shows,
from `views.list`: Inbox, Today, All, Scheduled and Done, then your lists (under their group
headings, or "Lists") and the Next 3 Days smart list, each a button with its colour or icon, the
number due today and the number open. Tapping one opens that view in the same tab, with a
**Views** button above the title to go back; tapping the Views tab again also goes back. The
phone remembers the opened view in this browser (not on the server) and shows it again next
time. If that list is deleted, the phone returns to the Views list and says "That list is
gone". Tapping a task opens its screen with its items (a checklist or subtasks); items
can be ticked and unticked there, with the same Undo banner. A new task from any view goes to
that view's capture list, as in the desktop; the New task sheet opens full screen. Both
`views.get` and `views.list` are in the allowlist of both roles (phone and display); so is
`tasks.get` (one task and its children). Only the phone role may write: it may complete and
reopen any task and capture plain tasks.

Done is not paged: `views.get` returns every completed task in one result. Measured on the
test fixture, a completed row is about 270 bytes of JSON, plus a few hundred bytes for the
rest of the result, so a few thousand completed tasks come to roughly a megabyte per fetch.

## The sync warning

When the daemon's sync needs attention (the server unreachable, a failing sync, a list that
cannot be discovered), the phone shows a red strip under the view title with a headline and a
detail line, for example the next retry time. It comes from the `sync` summary in `views.get`,
clears itself on the next refresh once sync is healthy, and has no close button. Nothing can be
fixed from the phone: act on it with `tdo` on the machine running the daemon.

## Logs

The gateway writes all its log lines (pairing, writes) to standard error: with Docker use
`docker compose logs -f twydo`. Without Docker they go to your terminal or service manager, and
the daemon logs to `$XDG_STATE_HOME/twydo/daemon-<socket-id>.log`.

The daemon logs sync state changes, not every run, one `tdo daemon:` line each (the same
lines land in that file). Lists appear by collection id, never by name:

```text
tdo daemon: sync ok (3 lists)
tdo daemon: sync failed (attempt 2, next in 120s): <error>
tdo daemon: sync ok again after 4 failures
tdo daemon: list discovery failed: <error>
tdo daemon: applied list <id> emptied on the server
tdo daemon: list <id> emptied on the server; not applied (edits here not yet synced)
```

A failure is logged once per distinct error text, and a recovery once. An emptied list that
is applied is logged every time; one that is held is logged when it first appears.
