//! Desktop media auto-detection.
//!
//! Two complementary sources are used:
//!   1. The OS "Now Playing" session (Now Playing on macOS, SMTC on Windows,
//!      MPRIS on Linux) for title / artist / album / position / state.
//!   2. The foreground browser tab (and, on macOS, player apps via AppleScript)
//!      for canonical video / channel URLs that the OS session does not expose.

use std::process::Command as ProcessCommand;
use std::sync::mpsc;
use std::time::Duration;

use crate::bot_gateway;
use crate::discord_rpc;
use crate::presence::{DetectedMedia, presence_from_detected};
use crate::scrobbling;
use crate::urls::{clean_title, inspect_url, split_artist_title};
use crate::MainWindow;

/// Commands accepted by the detector worker.
#[derive(Debug)]
pub enum Control {
    /// Begin polling every few seconds and keep Discord updated.
    Start,
    /// Stop polling (a manual "Detect now" still works).
    Stop,
    /// Run a single detection pass immediately.
    DetectNow,
    /// Replace the user-configured source toggles / behaviour.
    Configure(Settings),
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub native: bool,
    pub browser: bool,
    pub players: bool,
    pub auto_update: bool,
    pub activity_type: String,
    pub show_album: bool,
    pub hide_paused: bool,
    pub status_display_type: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            native: true,
            browser: true,
            players: true,
            auto_update: false,
            activity_type: "Auto".into(),
            show_album: true,
            hide_paused: false,
            status_display_type: "Details".into(),
        }
    }
}

/// Spawn the detector loop. `ui` is only ever touched through the Slint event
/// loop, so it is safe to call from this worker thread.
pub fn run_worker(
    rx: mpsc::Receiver<Control>,
    ui: slint::Weak<MainWindow>,
    rpc_tx: mpsc::Sender<discord_rpc::Command>,
    gateway_tx: mpsc::Sender<bot_gateway::Command>,
    scrobble_tx: mpsc::Sender<scrobbling::Command>,
) {
    let mut settings = Settings::default();
    let mut running = false;
    let mut last_signature = String::new();

    loop {
        let control = if running {
            match rx.recv_timeout(Duration::from_secs(3)) {
                Ok(control) => Some(control),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(control) => Some(control),
                Err(..) => break,
            }
        };

        let mut detect_now = false;
        match control {
            Some(Control::Start) => running = true,
            Some(Control::Stop) => running = false,
            Some(Control::DetectNow) => detect_now = true,
            Some(Control::Configure(next)) => settings = next,
            None => {}
        }

        if !running && !detect_now {
            continue;
        }

        set_busy(&ui, true, "Scanning for media…".to_string());
        let detected = detect_all(&settings);
        let status = detected.status.clone();
        set_busy(&ui, false, status);
        publish_ui(&ui, &detected);

        let signature = detection_signature(&detected);
        let should_update = signature != last_signature && !detected.is_empty();
        last_signature = signature;

        if should_update && (settings.auto_update || detect_now) {
            let skip_paused = settings.hide_paused && !detected.playing;
            if !skip_paused {
                if let Ok(presence) = presence_from_detected(
                    &detected,
                    &settings.activity_type,
                    settings.show_album,
                    &settings.status_display_type,
                ) {
                    let _ = rpc_tx.send(discord_rpc::Command::Update(presence.clone()));
                    let _ = gateway_tx.send(bot_gateway::Command::UpdatePresence(presence));
                    let _ = scrobble_tx.send(scrobbling::Command::SetTrack(Some(ScrobbleTrack {
                        title: detected.title.clone(),
                        artist: detected.artist.clone(),
                        album: detected.album.clone(),
                        duration_seconds: (detected.duration_seconds > 0)
                            .then_some(detected.duration_seconds),
                        position_seconds: detected.position_seconds,
                        playing: detected.playing,
                    })));
                }
            }
        }
    }
}

use crate::scrobbling::ScrobbleTrack;

fn detection_signature(media: &DetectedMedia) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        media.source,
        media.app,
        media.title,
        media.artist,
        media.album,
        media.playing,
        media.video_url
    )
}

fn set_busy(ui: &slint::Weak<MainWindow>, busy: bool, status: String) {
    let ui = ui.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            window.set_detecting(busy);
            window.set_detection_status(status.into());
        }
    });
}

fn publish_ui(ui: &slint::Weak<MainWindow>, media: &DetectedMedia) {
    let ui = ui.clone();
    let media = media.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            window.set_media_source(media.source.into());
            window.set_detected_app(media.app.into());
            window.set_media_title(media.title.into());
            window.set_media_artist(media.artist.into());
            window.set_media_album(media.album.into());
            window.set_media_kind(
                if media.watching {
                    "Music video"
                } else {
                    "Song"
                }
                .into(),
            );
            window.set_media_duration(media.duration_seconds.to_string().into());
            window.set_media_position(media.position_seconds.to_string().into());
            window.set_media_playing(media.playing);
            window.set_media_video_url(media.video_url.into());
            window.set_media_channel_url(media.channel_url.into());
            window.set_detected_site(media.site.into());
        }
    });
}

/// Run every enabled source and merge the best result.
pub fn detect_all(settings: &Settings) -> DetectedMedia {
    let mut candidates = Vec::new();

    if settings.native {
        if let Some(media) = native_now_playing() {
            candidates.push(media);
        }
    }
    if settings.players {
        if let Some(media) = player_apps() {
            candidates.push(media);
        }
    }
    if settings.browser {
        if let Some(media) = browser_tab() {
            candidates.push(media);
        }
    }

    if candidates.is_empty() {
        return DetectedMedia {
            status: "No media detected. Play something in a supported app or browser.".into(),
            ..DetectedMedia::default()
        };
    }

    candidates.sort_by_key(|media| std::cmp::Reverse(score(media)));
    let mut best = candidates.remove(0);
    for other in candidates {
        if best.title.is_empty() {
            best.title = other.title;
        }
        if best.artist.is_empty() {
            best.artist = other.artist;
        }
        if best.album.is_empty() {
            best.album = other.album;
        }
        if best.duration_seconds == 0 {
            best.duration_seconds = other.duration_seconds;
        }
        if best.position_seconds == 0 {
            best.position_seconds = other.position_seconds;
        }
        if best.video_url.is_empty() {
            best.video_url = other.video_url;
        }
        if best.channel_url.is_empty() {
            best.channel_url = other.channel_url;
        }
        if best.site.is_empty() {
            best.site = other.site;
        }
    }
    best.status = format!("Detected via {} · {}", best.source, best.app);
    best
}

fn score(media: &DetectedMedia) -> u32 {
    let mut value = 0;
    if !media.video_url.is_empty() {
        value += 100;
    }
    if !media.channel_url.is_empty() {
        value += 20;
    }
    if !media.title.is_empty() {
        value += 10;
    }
    if !media.artist.is_empty() {
        value += 5;
    }
    if media.playing {
        value += 2;
    }
    value
}

#[cfg(any(target_os = "linux", test))]
fn prefer_playing_media(
    candidates: impl IntoIterator<Item = DetectedMedia>,
) -> Option<DetectedMedia> {
    let mut paused_session = None;
    for candidate in candidates {
        if candidate.playing {
            return Some(candidate);
        }
        paused_session.get_or_insert(candidate);
    }
    paused_session
}

#[cfg(any(target_os = "linux", test))]
fn mpris_duration_seconds(microseconds: i64) -> u64 {
    (microseconds / 1_000_000).max(0) as u64
}

fn run(program: &str, args: &[&str]) -> Option<String> {
    let output = ProcessCommand::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Collapse the multiline text emitted by the AppleScript queries.
fn from_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim().to_string())
        .collect::<Vec<_>>()
}

fn line(lines: &[String], index: usize) -> String {
    lines.get(index).cloned().unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn osascript(script: &str) -> Option<String> {
    run("/usr/bin/osascript", &["-e", script])
}

// ---------------------------------------------------------------------------
// Native Now Playing session
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn native_now_playing() -> Option<DetectedMedia> {
    use block2::{Block, RcBlock};
    use dispatch2::{DispatchQoS, DispatchQueue, GlobalQueueIdentifier};
    use objc2::runtime::NSObject;
    use objc2_foundation::{NSDictionary, NSNumber, NSString};
    use std::ffi::c_void;
    use std::sync::{OnceLock, mpsc};

    type GetNowPlayingInfo = unsafe extern "C" fn(*mut c_void, *mut Block<dyn Fn(*mut c_void)>);

    static MEDIA_REMOTE: OnceLock<Option<libloading::Library>> = OnceLock::new();
    let library = MEDIA_REMOTE.get_or_init(|| unsafe {
        libloading::Library::new(
            "/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote",
        )
        .ok()
    });
    let library = library.as_ref()?;
    let get_info = unsafe {
        library
            .get::<GetNowPlayingInfo>(b"MRMediaRemoteGetNowPlayingInfo\0")
            .ok()?
    };

    let (sender, receiver) = mpsc::sync_channel(1);
    let callback: RcBlock<dyn Fn(*mut c_void)> = RcBlock::new(move |dictionary: *mut c_void| {
        if dictionary.is_null() {
            let _ = sender.send(None);
            return;
        }

        let info = unsafe { &*dictionary.cast::<NSDictionary<NSString, NSObject>>() };
        let get_string = |keys: &[&str]| -> String {
            keys.iter()
                .find_map(|key| {
                    let key = NSString::from_str(key);
                    info.objectForKey(&key)
                        .and_then(|value| value.downcast_ref::<NSString>().map(|s| s.to_string()))
                })
                .unwrap_or_default()
        };
        let get_number = |keys: &[&str]| -> Option<f64> {
            keys.iter().find_map(|key| {
                let key = NSString::from_str(key);
                info.objectForKey(&key)
                    .and_then(|value| value.downcast_ref::<NSNumber>().map(|n| n.as_f64()))
            })
        };
        let snapshot = (
            get_string(&["kMRMediaRemoteNowPlayingInfoTitle", "title"]),
            get_string(&["kMRMediaRemoteNowPlayingInfoArtist", "artist"]),
            get_string(&["kMRMediaRemoteNowPlayingInfoAlbum", "albumTitle", "album"]),
            get_number(&["kMRMediaRemoteNowPlayingInfoDuration", "playbackDuration"])
                .unwrap_or(0.0),
            get_number(&["kMRMediaRemoteNowPlayingInfoElapsedTime", "elapsedTime"]).unwrap_or(0.0),
            get_number(&["kMRMediaRemoteNowPlayingInfoPlaybackRate", "playbackRate"])
                .unwrap_or(0.0),
        );
        let _ = sender.send(Some(snapshot));
    });
    let queue = DispatchQueue::global_queue(GlobalQueueIdentifier::QualityOfService(
        DispatchQoS::UserInitiated,
    ));
    unsafe {
        get_info(
            dispatch2::DispatchRetained::as_ptr(&queue).as_ptr().cast(),
            RcBlock::as_ptr(&callback),
        );
    }

    let (raw_title, artist, album, duration, position, playback_rate) =
        receiver.recv_timeout(Duration::from_millis(750)).ok()??;
    let title = clean_title(&raw_title);
    if title.is_empty() {
        return None;
    }
    let duration = duration.max(0.0) as u64;
    let position = position.max(0.0) as u64;

    Some(DetectedMedia {
        source: "Now Playing".into(),
        app: "System".into(),
        title,
        artist,
        album,
        duration_seconds: duration,
        position_seconds: position.min(duration),
        playing: playback_rate > 0.0,
        ..DetectedMedia::default()
    })
}

#[cfg(target_os = "windows")]
fn native_now_playing() -> Option<DetectedMedia> {
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    };

    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .ok()?
        .join()
        .ok()?;
    let session = manager.GetCurrentSession().ok()?;
    let properties = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let title = clean_title(&properties.Title().ok()?.to_string_lossy());
    if title.is_empty() {
        return None;
    }
    let artist = properties
        .Artist()
        .ok()
        .map(|value| value.to_string_lossy())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            properties
                .AlbumArtist()
                .ok()
                .map(|value| value.to_string_lossy())
        })
        .unwrap_or_default();
    let album = properties
        .AlbumTitle()
        .map(|value| value.to_string_lossy())
        .unwrap_or_default();

    let mut duration = 0u64;
    let mut position = 0u64;
    if let Ok(timeline) = session.GetTimelineProperties() {
        // Windows TimeSpan is expressed in 100-nanosecond ticks.
        duration = (timeline
            .EndTime()
            .map(|value| value.Duration.max(0) / 10_000_000)
            .unwrap_or(0)) as u64;
        position = (timeline
            .Position()
            .map(|value| value.Duration.max(0) / 10_000_000)
            .unwrap_or(0)) as u64;
    }
    let playing = session
        .GetPlaybackInfo()
        .ok()
        .and_then(|info| info.PlaybackStatus().ok())
        == Some(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing);

    Some(DetectedMedia {
        source: "Now Playing".into(),
        app: session
            .SourceAppUserModelId()
            .ok()
            .map(|value| value.to_string_lossy())
            .unwrap_or_else(|| "System".into()),
        title,
        artist,
        album,
        duration_seconds: duration,
        position_seconds: position.min(duration),
        playing,
        ..DetectedMedia::default()
    })
}

#[cfg(target_os = "linux")]
#[zbus::proxy(
    interface = "org.mpris.MediaPlayer2.Player",
    default_path = "/org/mpris/MediaPlayer2"
)]
trait MprisPlayer {
    #[zbus(property)]
    fn metadata(&self) -> zbus::Result<zvariant::OwnedValue>;
    #[zbus(property)]
    fn playback_status(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn position(&self) -> zbus::Result<i64>;
}

#[cfg(target_os = "linux")]
fn native_now_playing() -> Option<DetectedMedia> {
    use zbus::blocking::Connection;
    use zvariant::Value;

    let connection = Connection::session().ok()?;
    let names = connection
        .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &())
        .ok()?;
    let names: Vec<String> = names.body().deserialize().ok()?;
    let mut candidates = Vec::new();
    for service in names
        .into_iter()
        .filter(|name| name.starts_with("org.mpris.MediaPlayer2."))
    {
        let Ok(proxy) = MprisPlayerProxyBlocking::new(&connection, service.as_str()) else {
            continue;
        };
        let Ok(metadata) = proxy.metadata() else {
            continue;
        };
        let Ok(dict) = metadata.downcast_ref::<zvariant::Dict<'_, '_>>() else {
            continue;
        };
        let lookup = |key: &str| -> Option<String> {
            let value = dict.get::<&str, Value<'_>>(&key).ok().flatten()?;
            match value {
                Value::Str(text) => Some(text.to_string()),
                Value::I64(number) => Some(number.to_string()),
                Value::U64(number) => Some(number.to_string()),
                Value::Array(array) => array
                    .iter()
                    .find_map(|value| value.downcast_ref::<String>().ok()),
                _ => None,
            }
        };

        let title = clean_title(&lookup("xesam:title").unwrap_or_default());
        if title.is_empty() {
            continue;
        }
        let duration = lookup("mpris:length")
            .and_then(|value| value.parse::<i64>().ok())
            .map(mpris_duration_seconds)
            .unwrap_or(0);
        let position = proxy.position().unwrap_or(0).max(0) as u64 / 1_000_000;
        let playback_status = proxy.playback_status().unwrap_or_default();
        if playback_status != "Playing" && playback_status != "Paused" {
            continue;
        }
        let playing = playback_status == "Playing";
        let artist = lookup("xesam:artist")
            .or_else(|| lookup("xesam:albumArtist"))
            .unwrap_or_default();
        let album = lookup("xesam:album").unwrap_or_default();
        let media = DetectedMedia {
            source: "Now Playing".into(),
            app: service
                .trim_start_matches("org.mpris.MediaPlayer2.")
                .to_string(),
            title,
            artist,
            album,
            artwork_url: lookup("mpris:artUrl").unwrap_or_default(),
            duration_seconds: duration,
            position_seconds: position.min(duration),
            playing,
            ..DetectedMedia::default()
        };
        candidates.push(media);
    }
    prefer_playing_media(candidates)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn native_now_playing() -> Option<DetectedMedia> {
    None
}

// ---------------------------------------------------------------------------
// Browser tabs (canonical video / channel links)
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn browser_tab() -> Option<DetectedMedia> {
    let browsers = [
        ("Google Chrome", "Chrome"),
        ("Safari", "Safari"),
        ("Microsoft Edge", "Edge"),
        ("Brave Browser", "Brave"),
        ("Arc", "Arc"),
        ("Firefox", "Firefox"),
    ];
    for (app, label) in browsers {
        let script = if app == "Firefox" {
            format!(
                "tell application \"{app}\"\nif it is running then\nreturn (URL of front document) & linefeed & (title of front document)\nend if\nend tell"
            )
        } else {
            format!(
                "tell application \"{app}\"\nif it is running then\nset activeTab to active tab of front window\nreturn (URL of activeTab) & linefeed & (title of activeTab)\nend if\nend tell"
            )
        };
        let Some(text) = osascript(&script) else { continue };
        let lines = from_lines(&text);
        let url = line(&lines, 0);
        let title = line(&lines, 1);
        let Some(info) = inspect_url(&url) else { continue };

        let cleaned = clean_title(&title);
        let (mut artist, mut track) = split_artist_title(&cleaned);
        if track.is_empty() {
            track = cleaned;
            artist.clear();
        }
        return Some(DetectedMedia {
            source: "Browser".into(),
            app: label.into(),
            title: track,
            artist,
            video_url: info.video_url,
            channel_url: info.channel_url,
            site: info.site,
            watching: info.watching,
            ..DetectedMedia::default()
        });
    }
    None
}

#[cfg(target_os = "windows")]
fn browser_tab() -> Option<DetectedMedia> {
    let script = "Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;using System.Text;public class W{[DllImport(\"user32.dll\")]public static extern IntPtr GetForegroundWindow();[DllImport(\"user32.dll\")]public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);}';$h=[W]::GetForegroundWindow();$sb=New-Object System.Text.StringBuilder 512;[void][W]::GetWindowText($h,$sb,512);$sb.ToString()";
    let title = run("powershell", &["-NoProfile", "-NonInteractive", "-Command", script])?;
    title_from_window(&title)
}

#[cfg(target_os = "linux")]
fn browser_tab() -> Option<DetectedMedia> {
    let title = run("xdotool", &["getactivewindow", "getwindowname"])
        .or_else(|| run("wmctrl", &["-l"]))?;
    title_from_window(&title)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn browser_tab() -> Option<DetectedMedia> {
    None
}

/// Shared fallback for Windows/Linux where the full tab URL is not exposed:
/// build a canonical search link from the window title when possible.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn title_from_window(raw: &str) -> Option<DetectedMedia> {
    let lower = raw.to_ascii_lowercase();
    let site = if lower.contains("youtube music") {
        "YouTube Music"
    } else if lower.contains("youtube") {
        "YouTube"
    } else if lower.contains("spotify") {
        "Spotify"
    } else if lower.contains("vimeo") {
        "Vimeo"
    } else if lower.contains("twitch") {
        "Twitch"
    } else {
        return None;
    };

    let cleaned = clean_title(raw);
    let (artist, title) = split_artist_title(&cleaned);
    let query = [&title, &artist]
        .iter()
        .filter(|value| !value.is_empty())
        .map(|value| value.replace(' ', "+"))
        .collect::<Vec<_>>()
        .join("+");
    let video_url = match site {
        "YouTube Music" => format!("https://music.youtube.com/search?q={query}"),
        "Spotify" => format!("https://open.spotify.com/search/{query}"),
        _ => format!("https://www.youtube.com/results?search_query={query}"),
    };

    Some(DetectedMedia {
        source: "Browser".into(),
        app: site.into(),
        title: if title.is_empty() { cleaned } else { title },
        artist,
        video_url,
        site: site.into(),
        watching: site != "Spotify" && site != "YouTube Music",
        ..DetectedMedia::default()
    })
}

// ---------------------------------------------------------------------------
// Player apps (AppleScript on macOS; covered by SMTC/MPRIS elsewhere)
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn player_apps() -> Option<DetectedMedia> {
    let spotify = "tell application \"Spotify\"\nif it is running then\nset tr to current track\nreturn (name of tr) & linefeed & (artist of tr) & linefeed & (album of tr) & linefeed & ((position of tr) as text) & linefeed & ((duration of tr) as text) & linefeed & ((player state) as text) & linefeed & ((id of tr) as text)\nend if\nend tell";
    let music = "tell application \"Music\"\nif it is running then\nset tr to current track\nreturn (name of tr) & linefeed & (artist of tr) & linefeed & (album of tr) & linefeed & ((position of tr) as text) & linefeed & ((duration of tr) as text) & linefeed & ((player state) as text)\nend if\nend tell";
    let itunes = music.replace("\"Music\"", "\"iTunes\"");

    if let Some(text) = osascript(spotify) {
        let lines = from_lines(&text);
        let title = clean_title(&line(&lines, 0));
        if !title.is_empty() {
            let id = line(&lines, 6);
            let track_id = id.strip_prefix("spotify:track:").unwrap_or("");
            let artist = line(&lines, 1);
            let video_url = if track_id.is_empty() {
                String::new()
            } else {
                format!("https://open.spotify.com/track/{track_id}")
            };
            let channel_url = if artist.is_empty() {
                String::new()
            } else {
                format!("https://open.spotify.com/search/{}", artist.replace(' ', "%20"))
            };
            let duration = line(&lines, 4).parse::<u64>().unwrap_or(0);
            let position = line(&lines, 3).parse::<u64>().unwrap_or(0);
            return Some(DetectedMedia {
                source: "Player".into(),
                app: "Spotify".into(),
                title,
                artist,
                album: line(&lines, 2),
                duration_seconds: duration,
                position_seconds: position.min(duration),
                playing: line(&lines, 5) == "playing",
                video_url,
                channel_url,
                site: "Spotify".into(),
                ..DetectedMedia::default()
            });
        }
    }

    for (script, app) in [(music, "Apple Music"), (itunes.as_str(), "iTunes")] {
        let Some(text) = osascript(script) else { continue };
        let lines = from_lines(&text);
        let title = clean_title(&line(&lines, 0));
        if title.is_empty() {
            continue;
        }
        let duration = line(&lines, 4).parse::<u64>().unwrap_or(0);
        let position = line(&lines, 3).parse::<u64>().unwrap_or(0);
        return Some(DetectedMedia {
            source: "Player".into(),
            app: app.into(),
            title,
            artist: line(&lines, 1),
            album: line(&lines, 2),
            duration_seconds: duration,
            position_seconds: position.min(duration),
            playing: line(&lines, 5) == "playing",
            ..DetectedMedia::default()
        });
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn player_apps() -> Option<DetectedMedia> {
    None
}

#[cfg(test)]
mod tests {
    use super::{
        detect_all, mpris_duration_seconds, prefer_playing_media, score, Settings,
    };
    use crate::presence::DetectedMedia;

    #[test]
    fn empty_detection_reports_help() {
        let media = detect_all(&Settings {
            native: false,
            browser: false,
            players: false,
            ..Settings::default()
        });
        assert!(media.is_empty());
        assert!(!media.status.is_empty());
    }

    #[test]
    fn scoring_prefers_links_over_bare_metadata() {
        let bare = DetectedMedia {
            title: "Song".into(),
            playing: true,
            ..DetectedMedia::default()
        };
        let linked = DetectedMedia {
            title: "Song".into(),
            video_url: "https://www.youtube.com/watch?v=abc".into(),
            ..DetectedMedia::default()
        };
        assert!(score(&linked) > score(&bare));
    }

    #[test]
    fn playing_mpris_session_takes_precedence_over_paused_session() {
        let paused = DetectedMedia {
            app: "paused-player".into(),
            title: "Paused track".into(),
            playing: false,
            ..DetectedMedia::default()
        };
        let playing = DetectedMedia {
            app: "playing-player".into(),
            title: "Playing track".into(),
            playing: true,
            ..DetectedMedia::default()
        };

        assert_eq!(
            prefer_playing_media([paused, playing.clone()]),
            Some(playing)
        );
    }

    #[test]
    fn mpris_duration_converts_microseconds_and_clamps_negative_values() {
        assert_eq!(mpris_duration_seconds(125_000_000), 125);
        assert_eq!(mpris_duration_seconds(-1), 0);
    }
}
