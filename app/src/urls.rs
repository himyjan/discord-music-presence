//! Pure helpers that turn browser URLs and window titles into the metadata used
//! by Media Presence. Kept free of OS-specific code so the parsing can be unit
//! tested on every platform.

use url::Url;

/// What we managed to learn from a browser tab / media URL.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkInfo {
    /// Canonical URL for the item being played (video/track).
    pub video_url: String,
    /// Canonical URL for the uploader / artist / channel when available.
    pub channel_url: String,
    /// Short site label, e.g. "YouTube", "YouTube Music", "Vimeo".
    pub site: String,
    /// Whether this looks like video content (Watching) instead of audio (Listening).
    pub watching: bool,
}

/// Host suffix helpers — match `youtube.com`, `www.youtube.com`, `m.youtube.com`.
fn host_is(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn param(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// Inspect a URL and return canonical item/channel links where possible.
pub fn inspect_url(raw: &str) -> Option<LinkInfo> {
    let url = Url::parse(raw.trim()).ok()?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let mut info = LinkInfo::default();

    if host_is(&host, "youtube.com") || host_is(&host, "youtu.be") {
        info.site = if host_is(&host, "music.youtube.com") {
            "YouTube Music".into()
        } else {
            "YouTube".into()
        };
        info.watching = !host_is(&host, "music.youtube.com");

        let video_id = if host_is(&host, "youtu.be") {
            url.path_segments()
                .and_then(|mut segments| segments.next())
                .map(str::to_owned)
        } else {
            param(&url, "v")
        };

        if let Some(id) = video_id.filter(|id| !id.is_empty()) {
            let base = if host_is(&host, "music.youtube.com") {
                "https://music.youtube.com/watch"
            } else {
                "https://www.youtube.com/watch"
            };
            info.video_url = format!("{base}?v={id}");
        } else if !url.path().is_empty() && url.path() != "/" {
            let path = url.path().trim_end_matches('/').to_string();
            info.video_url = format!("https://www.youtube.com{path}");
        }

        let channel = param(&url, "channelId")
            .map(|id| format!("https://www.youtube.com/channel/{id}"))
            .or_else(|| {
                url.path_segments().and_then(|segments| {
                    let parts: Vec<&str> = segments.collect();
                    let first = *parts.first()?;
                    if let Some(handle) = first.strip_prefix('@') {
                        return Some(format!("https://www.youtube.com/@{handle}"));
                    }
                    match first {
                        "c" => parts
                            .get(1)
                            .map(|name| format!("https://www.youtube.com/c/{name}")),
                        "channel" => parts
                            .get(1)
                            .map(|id| format!("https://www.youtube.com/channel/{id}")),
                        _ => None,
                    }
                })
            });
        if let Some(channel) = channel {
            info.channel_url = channel;
        }
        return Some(info);
    }

    if host_is(&host, "vimeo.com") {
        info.site = "Vimeo".into();
        info.watching = true;
        if let Some(id) = url
            .path_segments()
            .and_then(|mut segments| segments.next())
            .filter(|id| id.bytes().all(|byte| byte.is_ascii_digit()))
        {
            info.video_url = format!("https://vimeo.com/{id}");
        }
        return Some(info);
    }

    if host_is(&host, "twitch.tv") {
        info.site = "Twitch".into();
        info.watching = true;
        let parts: Vec<&str> = url.path_segments().map(|s| s.collect()).unwrap_or_default();
        match parts.as_slice() {
            ["v", id, ..] => {
                info.video_url = format!("https://www.twitch.tv/videos/{id}");
                info.channel_url = param(&url, "channel")
                    .map(|name| format!("https://www.twitch.tv/{name}"))
                    .unwrap_or_default();
            }
            [channel, ..] if !channel.is_empty() => {
                info.channel_url = format!("https://www.twitch.tv/{channel}");
                info.video_url = info.channel_url.clone();
            }
            _ => {}
        }
        return Some(info);
    }

    if host_is(&host, "bilibili.com") {
        info.site = "Bilibili".into();
        info.watching = true;
        info.video_url = raw.trim().to_string();
        return Some(info);
    }

    if host_is(&host, "open.spotify.com") {
        info.site = "Spotify".into();
        info.video_url = raw.trim().to_string();
        let parts: Vec<&str> = url.path_segments().map(|s| s.collect()).unwrap_or_default();
        if let Some(kind) = parts.get(1)
            && (*kind == "track" || *kind == "album")
        {
            info.video_url = format!("https://open.spotify.com/{}/{}", parts[0], *kind);
        }
        return Some(info);
    }

    if host_is(&host, "soundcloud.com") {
        info.site = "SoundCloud".into();
        info.video_url = raw.trim().trim_end_matches('/').to_string();
        return Some(info);
    }

    None
}

/// Split a typical "Artist - Title" window/tab title into its two parts.
pub fn split_artist_title(raw: &str) -> (String, String) {
    let mut parts = raw.splitn(2, " - ");
    let first = parts.next().unwrap_or("").trim();
    let second = parts.next().map(str::trim);
    match second {
        Some(second) if !first.is_empty() && !second.is_empty() => {
            (first.to_string(), second.to_string())
        }
        _ => (String::new(), raw.trim().to_string()),
    }
}

/// Remove the site/branding noise browsers append to tab titles.
pub fn clean_title(raw: &str) -> String {
    let mut title = raw.trim().to_string();
    let suffixes = [
        " - YouTube Music",
        " | YouTube Music",
        " - YouTube",
        " | YouTube",
        " - Vimeo",
        " | Vimeo",
        " - Twitch",
        " | Twitch",
        " - Spotify",
        " | Spotify",
    ];
    for suffix in suffixes {
        if title.ends_with(suffix) {
            title.truncate(title.len() - suffix.len());
            break;
        }
    }
    // Drop leading / trailing play glyphs that some browsers and players add.
    let glyphs = ['▶', '⏸', '♫', '♪', '🎵'];
    title = title.trim_start_matches(glyphs).trim().to_string();
    title = title.trim_end_matches(glyphs).trim().to_string();
    // Remove very common official-video decorations.
    for marker in ["(Official Video)", "(Official Music Video)", "[Official Video]"] {
        if let Some(index) = title.find(marker) {
            title.replace_range(index..index + marker.len(), "");
        }
    }
    title.trim().trim_end_matches(" -").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{clean_title, inspect_url, split_artist_title};

    #[test]
    fn youtube_watch_link_gets_video_and_channel() {
        let info = inspect_url(
            "https://www.youtube.com/watch?v=abc123&channelId=UCxyz&t=42s",
        )
        .unwrap();
        assert_eq!(info.site, "YouTube");
        assert!(info.watching);
        assert_eq!(info.video_url, "https://www.youtube.com/watch?v=abc123");
        assert_eq!(info.channel_url, "https://www.youtube.com/channel/UCxyz");
    }

    #[test]
    fn youtu_be_short_link_is_canonicalized() {
        let info = inspect_url("https://youtu.be/abc123").unwrap();
        assert_eq!(info.video_url, "https://www.youtube.com/watch?v=abc123");
    }

    #[test]
    fn youtube_music_is_listening_not_watching() {
        let info = inspect_url("https://music.youtube.com/watch?v=abc").unwrap();
        assert_eq!(info.site, "YouTube Music");
        assert!(!info.watching);
    }

    #[test]
    fn youtube_handle_channel_is_extracted() {
        let info = inspect_url("https://www.youtube.com/@somechannel/videos").unwrap();
        assert_eq!(info.channel_url, "https://www.youtube.com/@somechannel");
    }

    #[test]
    fn vimeo_and_twitch_links() {
        let vimeo = inspect_url("https://vimeo.com/123456").unwrap();
        assert_eq!(vimeo.video_url, "https://vimeo.com/123456");
        let twitch = inspect_url("https://www.twitch.tv/somechannel").unwrap();
        assert_eq!(twitch.channel_url, "https://www.twitch.tv/somechannel");
    }

    #[test]
    fn non_media_urls_are_ignored() {
        assert!(inspect_url("file:///Users/me/song.mp3").is_none());
        assert!(inspect_url("not a url").is_none());
    }

    #[test]
    fn titles_are_cleaned_and_split() {
        assert_eq!(clean_title("Bohemian Rhapsody - YouTube"), "Bohemian Rhapsody");
        assert_eq!(clean_title("▶ Artist - Song 🎵"), "Artist - Song");
        let (artist, title) = split_artist_title("Daft Punk - Get Lucky");
        assert_eq!(artist, "Daft Punk");
        assert_eq!(title, "Get Lucky");
    }
}
