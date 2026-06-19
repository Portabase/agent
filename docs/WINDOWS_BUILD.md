# Building and running the agent on Windows

This guide covers building `portabase-agent` from source on Windows and
running it locally, including the workarounds needed for two
Windows-specific issues:

1. `openssl-sys` fails to build (native TLS dependency).
2. `select_pg_path` needs a PostgreSQL install that Windows doesn't ship
   at the Debian/Ubuntu path the code originally assumed (see PR
   "fix(postgres): make `select_pg_path` cross-platform").

## Prerequisites

- **Rust** (stable toolchain, MSVC target — the default on Windows):
  `rustup default stable-x86_64-pc-windows-msvc`
- **Visual Studio Build Tools 2022**, with the "Desktop development with
  C++" workload (provides `cl.exe`/`nmake.exe`, required to compile any
  native dependency).
- **PostgreSQL** for Windows (the official EDB installer), e.g. version
  16: https://www.postgresql.org/download/windows/
  Default install path: `C:\Program Files\PostgreSQL\16`.
- **Redis** for Windows. There is no official Windows build of Redis;
  this guide uses the community-maintained
  [redis-windows](https://github.com/redis-windows/redis-windows)
  distribution.
- **vcpkg**, to provide a prebuilt OpenSSL (see below — the `vendored`
  OpenSSL feature currently fails to build on Windows for this project).

## 1. OpenSSL via vcpkg

The `vendored` OpenSSL feature (building OpenSSL from source via
`openssl-sys`) is the usual cross-platform fallback, but it did not
build successfully in testing on Windows (`nmake`-related hang/build
failure). Using a prebuilt OpenSSL via **vcpkg** worked reliably
instead:

```powershell
git clone https://github.com/microsoft/vcpkg
.\vcpkg\bootstrap-vcpkg.bat
.\vcpkg\vcpkg install openssl:x64-windows
```

Then point the build at it:

```powershell
$env:OPENSSL_DIR = "C:\path\to\vcpkg\installed\x64-windows"
```

Set this permanently (System Properties → Environment Variables) if you
don't want to re-export it in every new shell.

> If you do want to retry the `vendored` route instead, you'll
> additionally need Perl (Strawberry Perl) and NASM on `PATH`. Not
> required when using the vcpkg approach above.

## 2. Build

```powershell
git clone https://github.com/Portabase/agent.git
cd agent
cargo build --release
```

`cargo build` (debug) also works for local testing; the commands below
assume a debug build (`target\debug\`) to match local development, swap
in `target\release\` for a release build.

### Binary name

The build currently produces `app.exe` (taken from the crate/package
name in `Cargo.toml`). For a clearer, branded executable, rename the
binary output to `portabase-agent.exe` by setting an explicit binary
name in `Cargo.toml`:

```toml
[[bin]]
name = "portabase-agent"
path = "src/main.rs"
```

After this change, the build output becomes
`target\debug\portabase-agent.exe` (or `target\release\...` for release
builds). The run instructions below use `portabase-agent.exe`; replace
with `app.exe` if you haven't made this change.

## 3. Run Redis

Download/clone [redis-windows](https://github.com/redis-windows/redis-windows)
and start it on a custom port (here `65515`, to avoid clashing with any
other local Redis instance on the default `6379`):

```powershell
redis-server.exe --port 65515
```

Keep this running in its own terminal window.

## 4. Configure environment variables

The agent reads its configuration from environment variables. Example
startup script (adjust paths/values for your machine):

```bat
set "TZ=Europe/Berlin"
set "POLLING=60"
set "APP_ENV=production"
set "DATA_PATH=D:\pg_tools\agent\target\debug\data"
rem set "DATABASES_CONFIG_FILE"
set "CELERY_BROKER_URL=redis://localhost:65515/"
call "C:\Program Files\PostgreSQL\16\pg_env.bat"
portabase-agent.exe
```

Notes:

- `pg_env.bat` (shipped with the PostgreSQL installer) sets up
  `PATH`/`PGBIN` and other Postgres environment variables for the
  current shell — convenient as an alternative or complement to setting
  `PG_BIN_DIR` manually (see the `select_pg_path` cross-platform fix).
- `DATA_PATH` should point to a writable directory; create it beforehand
  if it doesn't exist yet (`mkdir D:\pg_tools\agent\target\debug\data`).
- Save the block above as e.g. `run-agent.bat` next to the executable
  for repeatable local runs.

### Using a `.env` file instead

Manually `set`-ing variables in a batch file works, but is easy to lose
track of and doesn't play well with version control hygiene (secrets
end up in shell history). If the agent does not yet support loading a
`.env` file, consider adding support via the [`dotenvy`](https://docs.rs/dotenvy)
crate (maintained successor of `dotenv`) early in `main()`:

```rust
fn main() {
    // Loads variables from a `.env` file in the current directory, if
    // present. Existing environment variables are not overridden, so
    // this is safe to call even when variables are already set
    // externally (e.g. by a process manager or CI).
    let _ = dotenvy::dotenv();

    // ... existing startup code
}
```

```toml
[dependencies]
dotenvy = "0.15"
```

Example `.env` file (place next to the executable, do **not** commit
this file — add `.env` to `.gitignore`):

```env
TZ=Europe/Berlin
POLLING=60
APP_ENV=production
DATA_PATH=D:\pg_tools\agent\target\debug\data
CELERY_BROKER_URL=redis://localhost:65515/
```

Note that `.env` files are a convenient alternative to the `set`
commands above, but won't run `pg_env.bat` for you — either keep that
`call` in a small wrapper script, or set `PG_BIN_DIR` directly in the
`.env` file once the cross-platform `select_pg_path` fix is in place,
e.g.:

```env
PG_BIN_DIR=C:\Program Files\PostgreSQL\16\bin
```

## 5. Start the agent

With Redis running and the environment configured (either via the batch
script or a `.env` file):

```powershell
.\portabase-agent.exe
```

## Troubleshooting

- **`openssl-sys` build fails / hangs on `nmake`**: use the vcpkg
  approach above instead of the `vendored` feature.
- **`pg_dump`/`pg_restore` not found**: make sure either `pg_env.bat`
  was called in the current shell, `PG_BIN_DIR` is set, or PostgreSQL
  is installed at the default `C:\Program Files\PostgreSQL\<version>\bin`
  location.
- **Redis connection refused**: confirm `redis-server.exe` is running
  and that `CELERY_BROKER_URL` matches the port it's listening on.
