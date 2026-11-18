# Proposed Rust + Slint Project

This document describes a proposed Rust and Slint implementation of Music
Presence. The initial desktop UI scaffold is in [`app/`](../app/README.md), but
it is not a description of the current application architecture or a
feature-complete application. The source code needed to validate existing
implementation details is not present in this checkout.

## Build the scaffold

From the [`app/`](../app/) directory, run `cargo run` to open the UI or
`cargo build --release` to produce an optimized binary. The current scaffold
has not yet been validated on every target operating system.

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

## Scaffold and planned structure

```text
app/
  Cargo.toml
  build.rs
  src/
    main.rs
  ui/
    main.slint
```

The Slint files define the windows and user interactions. Rust owns the
application lifecycle, integrations, persistence and background work. The UI
should receive small display models and send user actions through the generated
Slint APIs; it should not perform media I/O, network requests or image
conversion. The current scaffold contains only the window and app entry point;
the following modules are future work as features are ported:

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
  Discord Rich Presence updates through the local Discord desktop IPC and
  handle connection lifecycle away from the UI thread. This route requires the
  Discord desktop client; it is not an Android Gateway connection.
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

The current scaffold supports manually entered activity text over local
Discord desktop IPC, but does not implement the feature list above. Before it
can replace the existing application, implement and test feature parity,
settings migration, packaging, updates, media detection, tray behavior and
Discord connectivity on every target in the table. Do not remove the existing
implementation until parity and resource goals have been measured and reviewed
on all targets.
