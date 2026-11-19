missing# Music Presence — Rust + Slint preview

This is a desktop preview of the RPC features, not a feature-complete
replacement for Music Presence. The **Manual RPC** page supports manually
entered Custom RPC and Media RPC, activity links, and timeline metadata.
Media-player detection, playback control, player settings, album-art retrieval,
the system tray and updates are not implemented.

## Discord connections

- **Desktop Rich Presence:** Connects through Discord's local IPC using a
  public Application ID. Discord desktop must be running; no account token or
  login is required.
- **Bot Gateway:** Logs in as a Discord bot and updates the bot's activity.
  This does not change a user's personal profile. Login and logout are
  supported. If "Remember bot token" is enabled, AES-256-GCM encrypted token
  data and its key are stored separately in the operating system credential
  store; there is no plaintext fallback.
- **OAuth2 account sign-in:** Opens Discord in the system browser and uses
  authorization code with PKCE and the `identify` scope to show a profile
  preview with the display name, username, user ID, avatar and banner. It uses
  Kizzy Enhanced's OAuth Client ID by default; enter your own Client ID to
  override it. The redirect URI is `kizzy://oauth2/callback`. On first sign-in,
  the app registers a per-user OS URL handler for `kizzy://`; this can replace
  an existing default handler for that scheme. The handler relays the callback
  to the running app. OAuth2 is separate from bot login and does not update a user's
  personal activity. The saved access and refresh tokens are stored in the
  operating system credential store; logout attempts to revoke the token and
  clears the local session, reporting any credential-store or revocation
  errors.

OAuth2 credentials are not bot tokens and are never passed to the bot Gateway.
Personal Rich Presence uses Discord desktop IPC; bot activity uses a bot token.

Media metadata is entered manually. Turning off the Playing option marks the
activity as paused and removes the RPC timeline; it does not pause the media
player. The YouTube Music helper creates a search URL from the entered title,
artist, album or media kind; it does not resolve a canonical song, video,
album, playlist or channel URL.

To publish a manual activity, open **Manual RPC**, choose **Custom RPC**, fill
in the activity name and optional details/state, then connect to Discord
desktop with the public Application ID from the Discord Developer Portal.
Choose **Media RPC** instead to enter a track or video and its timeline
manually.

## Scrobbling

The **Scrobbling** page supports Last.fm, ListenBrainz and Libre.fm. Last.fm
and Libre.fm require an API key and secret from [Last.fm's developer page](https://www.last.fm/api/account/create)
or [Libre.fm's API key page](https://libre.fm/api-keys.php) (Libre.fm documents
32-character API keys); enter them and authorize in the browser, then return
to the app and choose **Finish sign-in**.
ListenBrainz uses a user token from
[your settings](https://listenbrainz.org/settings/). All session keys and
tokens are saved in the OS credential store.

Scrobbling uses the manually entered **Media RPC** track. Enter both a title
and artist, choose **Song** or **Music video**, and connect the desktop RPC or
bot Gateway. The app sends a now-playing update when the track starts, then
submits one listen after half the duration or four minutes, whichever comes
first; tracks shorter than 30 seconds are ignored. Update the connected
presence when you change tracks, seek or pause; automatic media-player
detection is not implemented.

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

## GitHub releases

Pushing a tag like `slint-v0.1.0` runs the multi-platform release workflow and
publishes a stable GitHub release with native packages for each target below.
Running the workflow manually builds packages without publishing a release.

The Linux x86-64 release includes `.AppImage`, `.deb` and `.rpm` packages.
macOS ARM64 and x86-64 releases include `.dmg` images. Windows x86-64 is
distributed as portable `.zip` and `.7z` archives containing the executable
and this README; both use 7-Zip's maximum compression setting.
The macOS app and Windows executable are unsigned; macOS may require opening
the app through Finder's **Open** command to approve it.
The shared SVG is used for the in-app logo, native window icons where
supported, Linux desktop launchers, and the macOS Dock icon.

## Current targets

The project is configured for Windows x86-64, macOS ARM64 and x86-64, and Linux
x86-64. The release workflow builds on native runners for each target. Each
build still needs runtime and feature validation on its target platform,
including the platform credential store used for encrypted token persistence.
