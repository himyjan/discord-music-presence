# Rust + Slint Preview and Migration Plan

The desktop preview in [`app/`](../app/README.md) implements manually entered
Custom RPC and Media RPC, Discord desktop IPC, Discord bot Gateway login and
activity updates, and AES-256-GCM credential persistence. It is not
feature-complete and does not provide automatic media detection, playback
control, tray behavior, updates or complete parity with the original
application.

## Build the preview

From the [`app/`](../app/) directory, run `cargo run` to open the UI or
`cargo build --release` to produce an optimized binary. Build and runtime
validation is still required on every target operating system.

## Goals

- Replace the desktop UI with [Slint](https://slint.dev/) while preserving the
  existing Music Presence features and user-visible behavior.
- Build and validate for every platform and architecture currently distributed.
- Keep the UI responsive while media detection, artwork handling, Discord
  communication and updates run independently.
- Keep idle CPU and memory use low, minimize unnecessary dependencies, and
  measure release package size on every supported platform.
- Keep platform-specific integrations isolated so each operating system can be
  tested and maintained independently.

## Current and planned structure

```text
app/
  Cargo.toml
  build.rs
  src/
    bot_gateway.rs
    discord_rpc.rs
    main.rs
    presence.rs
    token_store.rs
  ui/
    main.slint
```

The Slint files define the windows and user interactions. Rust owns the
application lifecycle, integrations, persistence and background work. The UI
should receive small display models and send user actions through the generated
Slint APIs; it should not perform media I/O, network requests or image
conversion. The preview keeps Discord IPC, Gateway and credential handling in
separate Rust modules. The following modules are future work as the remaining
features are ported:

```text
src/
  app.rs
  media/
  discord/
  settings.rs
  tray.rs
  updates.rs
ui/
  settings.slint
```

## Platform targets

The project should keep all currently published targets enabled:

| Operating system | Architecture | Package examples |
| --- | --- | --- |
| Windows | x86-64 | Installer and ZIP |
| macOS | ARM64 | DMG |
| macOS | x86-64 | DMG |
| Linux | x86-64 | AppImage, DEB, RPM and tarball |

Keep OS-specific media detection, tray integration, Discord IPC, update
installation and packaging behind platform adapters. Use shared Rust and Slint
code for behavior that is genuinely common, but do not assume platform APIs or
capabilities are identical. A feature should not be silently omitted on one
target: document any platform limitation and test it explicitly.

## Responsibilities

- **UI (`ui/`):** main/tray-facing status, player list and settings screens.
  Keep presentation state separate from platform integration details.
- **Media layer (`media/`):** detect active players, normalize metadata and
  report playback changes. Prefer operating-system events where available;
  use bounded, low-frequency refreshes only where event notifications are not
  available.
- **Discord layer (`discord/`):** convert normalized playback state into
  Discord Rich Presence updates through local desktop IPC or a bot Gateway
  connection, and handle connection lifecycle away from the UI thread. Desktop
  IPC requires the Discord desktop client; bot Gateway activity updates affect
  the bot, not a user's personal profile.
- **Settings (`settings.rs`):** load, validate and persist global and per-player
  preferences without blocking UI interactions.
- **Tray (`tray.rs`):** present active, waiting and disabled states using the
  platform's tray facilities.
- **Updates (`updates.rs`):** check and apply updates according to the existing
  update flow, with clear user-facing status and errors.

## Feature-preservation requirements

The migration should retain the current product behavior, including:

- Detecting the supported media players and showing their metadata and album
  artwork without relying on external artwork sources.
- Showing playback position when available, including the progress indicator.
- Choosing which apps are shared and how the status is formatted, with
  per-player overrides.
- Optionally retaining paused media in the status.
- Supporting optional animated artwork for Apple Music and TIDAL, with the
  existing default-off behavior and conversion behavior.
- Showing Spotify podcast status and allowing Spotify music to continue using
  the official Spotify integration.
- Providing supported "Play on" buttons and additional artist information
  where available.
- Reflecting active, waiting and disabled states in the tray.
- Preserving in-app/automatic updates and their user-facing notifications.

These are parity requirements, not claims that a new implementation already
supports them. Each should be verified against the existing application before
replacing it.

## Resource and package-size practices

- Keep all potentially blocking work off the Slint event loop. Use a small,
  bounded amount of background work rather than spawning work repeatedly.
- Avoid unnecessary polling and duplicate Discord updates. Compare normalized
  state before publishing a change.
- Bound artwork caching and decode images only at the sizes the UI needs.
- Keep animated artwork disabled by default; perform conversion only when the
  feature is explicitly enabled.
- Start with the smallest set of runtime dependencies that supports the
  required platforms and features. Evaluate optional dependencies against the
  package-size and feature-parity requirements before removing or gating them.
- Use optimized release builds and platform-appropriate packaging. Measure
  actual artifact sizes rather than relying on estimates; retain debug symbols
  separately when release debugging requires them.

## Verification before replacing the current application

Establish a baseline for startup time, idle and active CPU use, peak memory and
installed package size on each supported operating system. Compare the Slint
implementation under the same conditions, including media detection,
artwork loading, animation conversion and Discord reconnects.

The current preview supports manually entered activity over Discord desktop
IPC or a bot Gateway connection. Its media timeline uses user-entered position
and duration, and its paused state only changes the activity presentation; it
does not control a player. Before it can replace the existing application,
implement and test feature parity, settings migration, packaging, updates,
media detection, tray behavior and Discord connectivity on every target in the
table. Do not remove the existing implementation until parity and resource
goals have been measured and reviewed on all targets.
