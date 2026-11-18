# Music Presence — Rust + Slint scaffold

This directory contains a desktop Slint preview, not a feature-complete
replacement for Music Presence. It can publish a manually entered activity to
the running Discord desktop client using Discord's local Rich Presence IPC.
Media detection, player settings, album artwork, the system tray and updates
are not implemented yet.

The connection uses a public Discord Application ID, not a user account token
or a direct Discord Gateway login. Create an application in the Discord
Developer Portal, run Discord desktop, then enter its Application ID in the
preview. The Android-style Gateway connection described by Kizzy is not
implemented.

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

## Current targets

The project is configured for the platforms currently distributed by Music
Presence: Windows x86-64, macOS ARM64 and x86-64, and Linux x86-64. This
scaffold has only been built on the host platform so far; it still needs native
build and runtime validation on each listed target.
