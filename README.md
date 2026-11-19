![Music Presence running in the system tray and showing a Discord status](./assets/banner.png)

[![GitHub stars](https://img.shields.io/github/stars/himyjan/discord-music-presence?style=flat&label=Stars&logo=github&labelColor=444&color=DAAA3F&cacheSeconds=3600)](https://star-history.com/#himyjan/discord-music-presence&Date)
&nbsp;[![Total downloads](https://img.shields.io/github/downloads/himyjan/discord-music-presence/total?style=flat&label=Downloads&labelColor=444&logo=hack-the-box&logoColor=white&cacheSeconds=600)](https://github.com/himyjan/discord-music-presence/releases)
&nbsp;[![Latest release downloads](https://img.shields.io/github/downloads/himyjan/discord-music-presence/latest/total?style=flat&label=Downloads%20%40latest&labelColor=444&logo=hack-the-box&logoColor=white&cacheSeconds=600)](https://github.com/himyjan/discord-music-presence/releases/latest)
&nbsp;[![Discord community](https://img.shields.io/discord/1224509771068211292?logo=discord&logoColor=eee&label=Discord&labelColor=464ce5&color=fff&cacheSeconds=43200)](https://discord-invite.musicpresence.app)

# Music Presence

Share what you listen to on Discord. Music Presence detects playback from more
than 100 media players and displays it as a Rich Presence status, with controls
for what gets shared and how it appears.

It works with players including Apple Music, Spotify, TIDAL and foobar2000.
When playback is detected, the app appears in your system tray.

## Download and install

Get the latest release from the
[GitHub releases page](https://github.com/himyjan/discord-music-presence/releases/latest),
or choose a download for your platform:

<!-- DL_BUTTONS_BEGIN -->

[<img src="https://raw.githubusercontent.com/himyjan/discord-music-presence/refs/heads/master/assets/download_windows-installer-x64.png" width="190" alt="Windows x64 installer">](https://github.com/himyjan/discord-music-presence/releases/download/v2.3.6/musicpresence-2.3.6-windows-x64-installer.exe)
[<img src="https://raw.githubusercontent.com/himyjan/discord-music-presence/refs/heads/master/assets/download_windows-zip-x64.png" width="190" alt="Windows x64 ZIP">](https://github.com/himyjan/discord-music-presence/releases/download/v2.3.6/musicpresence-2.3.6-windows-x64.zip)
[<img src="https://raw.githubusercontent.com/himyjan/discord-music-presence/refs/heads/master/assets/download_mac-dmg-arm64.png" width="190" alt="macOS ARM64 disk image">](https://github.com/himyjan/discord-music-presence/releases/download/v2.3.6/musicpresence-2.3.6-mac-arm64.dmg)
[<img src="https://raw.githubusercontent.com/himyjan/discord-music-presence/refs/heads/master/assets/download_mac-dmg-x86_64.png" width="190" alt="macOS x86-64 disk image">](https://github.com/himyjan/discord-music-presence/releases/download/v2.3.6/musicpresence-2.3.6-mac-x86_64.dmg)
[<img src="https://raw.githubusercontent.com/himyjan/discord-music-presence/refs/heads/master/assets/download_linux-AppImage-x64.png" width="190" alt="Linux x86-64 AppImage">](https://github.com/himyjan/discord-music-presence/releases/download/v2.3.6/musicpresence-2.3.6-linux-x86_64.AppImage)
[<img src="https://raw.githubusercontent.com/himyjan/discord-music-presence/refs/heads/master/assets/download_linux-deb-x64.png" width="190" alt="Linux x86-64 DEB package">](https://github.com/himyjan/discord-music-presence/releases/download/v2.3.6/musicpresence-2.3.6-linux-x86_64.deb)
[<img src="https://raw.githubusercontent.com/himyjan/discord-music-presence/refs/heads/master/assets/download_linux-rpm-x64.png" width="190" alt="Linux x86-64 RPM package">](https://github.com/himyjan/discord-music-presence/releases/download/v2.3.6/musicpresence-2.3.6-linux-x86_64.rpm)
[<img src="https://raw.githubusercontent.com/himyjan/discord-music-presence/refs/heads/master/assets/download_linux-tar-gz-x64.png" width="190" alt="Linux x86-64 tarball">](https://github.com/himyjan/discord-music-presence/releases/download/v2.3.6/musicpresence-2.3.6-linux-x86_64.tar.gz)

<!-- DL_BUTTONS_END -->

Package-manager options:

- macOS: `brew install music-presence`
- Arch Linux (unofficial): `yay -S music-presence-bin`

See the [installation instructions](./documentation/installation-instructions.md)
for help choosing and installing a package. Choose the build that matches your
operating system and processor architecture.

## Features

- **Broad player support:** works with more than 100 media players. If yours is
  not detected, [see the troubleshooting guide](./documentation/troubleshooting.md#if-your-media-player-is-still-not-detected).
- **Accurate artwork:** shows the correct album cover for every supported
  player, without relying on external image sources.
- **Playback position:** shows the exact current position and progress bar when
  the player provides it.
- **Flexible status:** choose whether to show the artist, title, player or media
  type, and configure which applications may be shared.
- **Per-player settings:** use individual overrides for supported players.
- **Animated artwork:** available for Apple Music and TIDAL when explicitly
  enabled. Videos are converted on the fly and may take a few seconds to load.
- **Paused media:** optionally keep paused playback in your status for as long
  as you want.
- **Spotify podcasts:** show podcasts as "Listening to a Podcast" with Music
  Presence while leaving Spotify music to the official Spotify integration, or
  customize podcast sharing in Music Presence.
- **Player links and extra artists:** show a "Play on" button for Apple Music,
  TIDAL, Deezer and Spotify, and optionally show additional artists. Button
  availability depends on the player's API and may be affected by rate limits.
- **Tray status:** the tray icon indicates whether sharing is active, waiting
  for playback or disabled by you.
- **Updates:** in-app and automatic updates, with information about changes in
  new versions.
- **One place for your music status:** replace separate player-specific Rich
  Presence apps or plugins with shared controls and per-player customization.
- **Native-feeling Discord integration:** get the familiar Discord activity
  experience with additional controls and customization.

### Enhanced desktop UI and auto-detection

This desktop build adds a Discord-branded interface with dark and light themes
(based on [Discord's branding](https://discord.com/branding)) and a feature-
based navigation layout inspired by Kizzy Enhanced. The **Media RPC** page can
automatically detect playback from:

- the OS Now Playing session (**macOS** Now Playing, **Windows** SMTC, **Linux**
  MPRIS),
- the foreground browser tab, extracting canonical **video / track** and
  **channel / artist** URLs for YouTube, YouTube Music, Vimeo, Twitch, Bilibili,
  Spotify and SoundCloud, and
- local player apps on macOS (Spotify, Apple Music and iTunes) via AppleScript.

Press **Detect now** for a single scan, or enable **Auto-detect** to keep the
connected Discord activity in sync. When a full tab URL is unavailable, a
canonical search link is generated from the window title instead.

Detected links are also attached to the Rich Presence **details** and **state**
text and to the large image asset (Discord Social SDK field URLs), so the title,
artist/channel and artwork are clickable in Discord. The **Activity** page has
matching fields for manual activities.

### Keep optional processing off

Animated artwork is disabled by default. Leave it disabled if you do not need
video covers; enabling it converts them on the fly and may take a few seconds
to load. Keeping it off avoids that optional conversion step.

## Documentation

- [Installation and setup](./documentation/installation-instructions.md)
- [Supported media players](./documentation/supported-media-players.md)
- [Troubleshooting](./documentation/troubleshooting.md)
- [Third-party integration](./documentation/third-party-integration.md)
- [Verifying downloads](./documentation/automatic-updates.md#verifying-releases)
- [All documentation](./documentation/README.md)
- [Changelog](./CHANGELOG.md)
- [Roadmap](./documentation/roadmap.md)
- [Privacy](./PRIVACY.md)

For the project's plans for scrobbling, browser support and listening along,
see the [Timeline](https://musicpresence.app?utm_source=github.com&utm_medium=project-readme)
on the website.

## Community and support


## Improving media-player support

Some players provide incomplete metadata. For example, Chromium-based players
may not report album names through the Windows Media Session API. This issue
has been reported to Chromium at
[MediaSession API does not report the album to the Windows system](https://issues.chromium.org/issues/349310439)
and discussed with TIDAL
[here](https://github.com/orgs/tidal-music/discussions/74).

## Links

- [Website](https://musicpresence.app)
- [Discord server](https://discord-invite.musicpresence.app)
- [GitHub releases](https://github.com/himyjan/discord-music-presence/releases)
- [Star history](https://star-history.com/#himyjan/discord-music-presence&Date)

## Disclaimer

Music Presence is not affiliated with or endorsed by Discord, any streaming
service, media player or music distributor.

Copyright (c) 2024-2025 Jonas van den Berg
