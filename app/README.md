# Music Presence — Rust + Slint preview

This is a desktop preview of the RPC features, not a feature-complete
replacement for Music Presence. It supports manually entered Custom RPC and
Media RPC, activity links, and timeline metadata. Media-player detection,
playback control, player settings, album-art retrieval, the system tray and
updates are not implemented.

## Discord connections

- **Desktop Rich Presence:** Connects through Discord's local IPC using a
  public Application ID. Discord desktop must be running; no account token or
  login is required.
- **Bot Gateway:** Logs in as a Discord bot and updates the bot's activity.
  This does not change a user's personal profile. Login and logout are
  supported. If "Remember bot token" is enabled, AES-256-GCM encrypted token
  data and its key are stored separately in the operating system credential
  store; there is no plaintext fallback.

Pasted OAuth2 user tokens are intentionally unsupported: they are not bot
tokens and cannot be used to set a user's personal activity through this app.

Media metadata is entered manually. Turning off the Playing option marks the
activity as paused and removes the RPC timeline; it does not pause the media
player. The YouTube Music helper creates a search URL from the entered title,
artist, album or media kind; it does not resolve a canonical song, video,
album, playlist or channel URL.

## Requirements

- Rust 1.95 or newer
- A desktop development environment supported by Slint's Winit backend

## Build and run

From this directory:

```sh
cargo run
```

For an optimized release build:

```sh
cargo build --release
```

The release profile uses size-oriented optimization and link-time optimization.
Measure the generated package and runtime resource use on each target before
making performance claims.

## GitHub prereleases

Pushing a tag like `slint-v0.1.0` runs the multi-platform release workflow and
publishes a prerelease with archives for each target below. Running the
workflow manually builds the archives without publishing a release.

## Current targets

The project is configured for the platforms currently distributed by Music
Presence: Windows x86-64, macOS ARM64 and x86-64, and Linux x86-64. The release
workflow builds on native runners for each listed target. Each build still
needs runtime and feature validation on its target platform, including the
platform credential store used for encrypted token persistence.
