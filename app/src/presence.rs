use std::time::{SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{self, Activity, Assets, Button, Timestamps};
use twilight_model::gateway::presence::activity_button::ActivityButtonLink;
use twilight_model::gateway::presence::{
    Activity as GatewayActivity, ActivityAssets, ActivityButton, ActivityTimestamps, ActivityType,
};

const TEXT_LIMIT: usize = 128;
const BUTTON_LABEL_LIMIT: usize = 32;
const BUTTON_URL_LIMIT: usize = 512;

/// A snapshot of what the desktop detector currently sees playing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DetectedMedia {
    /// Detection backend that produced this snapshot ("Now Playing", "Browser", "Spotify", ...).
    pub source: String,
    /// Application or browser that owns the playback.
    pub app: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: u64,
    pub position_seconds: u64,
    pub playing: bool,
    /// Human-readable status message for the UI.
    pub status: String,
    /// Canonical item URL (video / track), when known.
    pub video_url: String,
    /// Canonical channel / artist URL, when known.
    pub channel_url: String,
    /// External artwork image URL, when the backend exposes one.
    pub artwork_url: String,
    /// Short site label used for the activity name and button text.
    pub site: String,
    /// Whether the detected item is video (Watching) rather than audio (Listening).
    pub watching: bool,
}

impl DetectedMedia {
    pub fn is_empty(&self) -> bool {
        self.title.trim().is_empty() && self.artist.trim().is_empty()
    }
}

/// Build a Discord activity from an auto-detected media snapshot.
///
/// `activity_type_override` accepts "Listening" / "Watching"; anything else
/// falls back to the media's own `watching` hint.
pub fn presence_from_detected(
    media: &DetectedMedia,
    activity_type_override: &str,
    show_album: bool,
) -> Result<PresenceDraft, String> {
    if media.title.trim().is_empty() && media.artist.trim().is_empty() {
        return Err("No media detected yet.".into());
    }

    let name = if !media.site.trim().is_empty() {
        media.site.trim().to_string()
    } else if !media.app.trim().is_empty() {
        media.app.trim().to_string()
    } else {
        "Music Presence".to_string()
    };

    let mut state_parts = Vec::new();
    if !media.artist.trim().is_empty() {
        state_parts.push(media.artist.trim().to_string());
    }
    if show_album && !media.album.trim().is_empty() {
        state_parts.push(media.album.trim().to_string());
    }
    if !media.playing {
        state_parts.push("Paused".to_string());
    }
    let state = state_parts.join(" · ");

    let activity_type = match activity_type_override {
        "Listening" => 2,
        "Watching" => 3,
        "Playing" => 0,
        _ if media.watching => 3,
        _ => 2,
    };

    let (button1_label, button1_url) = if media.video_url.trim().is_empty() {
        (String::new(), String::new())
    } else {
        let label = if media.site.eq_ignore_ascii_case("youtube music") {
            "Open in YouTube Music".to_string()
        } else if media.site.eq_ignore_ascii_case("spotify") {
            "Open in Spotify".to_string()
        } else if media.site.is_empty() {
            "Open media".to_string()
        } else {
            format!("Open in {}", media.site)
        };
        (label, media.video_url.trim().to_string())
    };

    let (button2_label, button2_url) = if media.channel_url.trim().is_empty() {
        (String::new(), String::new())
    } else {
        let label = if media.watching {
            "Visit channel"
        } else {
            "View artist"
        };
        (label.to_string(), media.channel_url.trim().to_string())
    };

    let draft = PresenceDraft {
        name,
        details: media.title.trim().to_string(),
        state,
        activity_type,
        playing: media.playing,
        position_seconds: media.position_seconds,
        duration_seconds: media.duration_seconds,
        large_image: media.artwork_url.trim().to_string(),
        large_text: media.album.trim().to_string(),
        details_url: media.video_url.trim().to_string(),
        state_url: media.channel_url.trim().to_string(),
        large_url: if media.artwork_url.trim().is_empty() {
            String::new()
        } else {
            media.video_url.trim().to_string()
        },
        small_url: String::new(),
        button1_label: truncate_button_label(button1_label),
        button1_url,
        button2_label: truncate_button_label(button2_label),
        button2_url,
    };
    draft.validate()?;
    Ok(draft)
}

fn truncate_button_label(label: String) -> String {
    if label.chars().count() <= 32 {
        return label;
    }
    label.chars().take(32).collect()
}

#[derive(Clone, Debug)]
pub struct PresenceDraft {
    pub name: String,
    pub details: String,
    pub state: String,
    pub activity_type: u8,
    pub playing: bool,
    pub position_seconds: u64,
    pub duration_seconds: u64,
    pub large_image: String,
    pub large_text: String,
    /// Clickable URL for the `details` line (Discord Social SDK field URLs).
    pub details_url: String,
    /// Clickable URL for the `state` line.
    pub state_url: String,
    /// Clickable URL for the large image asset.
    pub large_url: String,
    /// Clickable URL for the small image asset.
    pub small_url: String,
    pub button1_label: String,
    pub button1_url: String,
    pub button2_label: String,
    pub button2_url: String,
}

impl PresenceDraft {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Enter an activity name.".into());
        }
        if self.name.chars().count() > TEXT_LIMIT
            || self.details.chars().count() > TEXT_LIMIT
            || self.state.chars().count() > TEXT_LIMIT
            || self.large_text.chars().count() > TEXT_LIMIT
        {
            return Err(format!(
                "Activity text must be {TEXT_LIMIT} characters or fewer."
            ));
        }

        validate_button(&self.button1_label, &self.button1_url)?;
        validate_button(&self.button2_label, &self.button2_url)?;
        validate_link_url("Details URL", &self.details_url)?;
        validate_link_url("State URL", &self.state_url)?;
        validate_link_url("Large image URL", &self.large_url)?;
        validate_link_url("Small image URL", &self.small_url)?;

        if self.position_seconds > self.duration_seconds && self.duration_seconds > 0 {
            return Err("Playback position cannot exceed the track duration.".into());
        }

        if self.playing && self.duration_seconds > 0 {
            let start = unix_seconds().saturating_sub(self.position_seconds);
            let end_millis = start
                .checked_add(self.duration_seconds)
                .and_then(|seconds| seconds.checked_mul(1000))
                .ok_or_else(|| "Media timeline exceeds Discord's timestamp limits.".to_string())?;
            if end_millis > i64::MAX as u64 {
                return Err("Media timeline exceeds Discord's timestamp limits.".into());
            }
        }

        Ok(())
    }

    pub fn ipc_activity(&self) -> Activity<'_> {
        let mut activity =
            Activity::new()
                .name(self.name.trim())
                .activity_type(match self.activity_type {
                    2 => activity::ActivityType::Listening,
                    3 => activity::ActivityType::Watching,
                    5 => activity::ActivityType::Competing,
                    _ => activity::ActivityType::Playing,
                });

        if !self.details.trim().is_empty() {
            activity = activity.details(self.details.trim());
        }
        if !self.details_url.trim().is_empty() {
            activity = activity.details_url(self.details_url.trim());
        }
        if !self.state.trim().is_empty() {
            activity = activity.state(self.state.trim());
        }
        if !self.state_url.trim().is_empty() {
            activity = activity.state_url(self.state_url.trim());
        }
        if !self.large_image.trim().is_empty() {
            let mut assets = Assets::new()
                .large_image(self.large_image.trim())
                .large_text(self.large_text.trim());
            if !self.large_url.trim().is_empty() {
                assets = assets.large_url(self.large_url.trim());
            }
            if !self.small_url.trim().is_empty() {
                assets = assets.small_url(self.small_url.trim());
            }
            activity = activity.assets(assets);
        }

        let buttons = [
            (&self.button1_label, &self.button1_url),
            (&self.button2_label, &self.button2_url),
        ]
        .into_iter()
        .filter(|(label, url)| !label.trim().is_empty() && !url.trim().is_empty())
        .map(|(label, url)| Button::new(label.trim(), url.trim()))
        .collect();
        activity = activity.buttons(buttons);

        if self.playing && self.duration_seconds > 0 {
            let now = unix_seconds();
            let start = now.saturating_sub(self.position_seconds);
            activity = activity.timestamps(
                Timestamps::new()
                    .start(start as i64 * 1000)
                    .end((start + self.duration_seconds) as i64 * 1000),
            );
        }

        activity
    }

    pub fn gateway_activity(&self) -> GatewayActivity {
        let now = unix_seconds();
        let timestamps = if self.playing && self.duration_seconds > 0 {
            let start = now.saturating_sub(self.position_seconds);
            Some(ActivityTimestamps {
                start: Some(start.saturating_mul(1000)),
                end: Some(
                    start
                        .saturating_add(self.duration_seconds)
                        .saturating_mul(1000),
                ),
            })
        } else {
            None
        };
        let buttons = [
            (&self.button1_label, &self.button1_url),
            (&self.button2_label, &self.button2_url),
        ]
        .into_iter()
        .filter(|(label, url)| !label.trim().is_empty() && !url.trim().is_empty())
        .map(|(label, url)| {
            ActivityButton::Link(ActivityButtonLink {
                label: label.trim().to_owned(),
                url: url.trim().to_owned(),
            })
        })
        .collect();

        let assets = (!self.large_image.trim().is_empty()).then(|| ActivityAssets {
            large_image: Some(self.large_image.trim().to_owned()),
            large_text: nonempty(&self.large_text),
            large_url: nonempty(&self.large_url),
            small_image: None,
            small_text: None,
            small_url: nonempty(&self.small_url),
            invite_cover_image: None,
        });

        GatewayActivity {
            application_id: None,
            assets,
            buttons,
            created_at: None,
            details: nonempty(&self.details),
            details_url: nonempty(&self.details_url),
            emoji: None,
            flags: None,
            id: None,
            instance: None,
            kind: ActivityType::from(self.activity_type),
            name: self.name.trim().to_owned(),
            party: None,
            secrets: None,
            state: nonempty(&self.state),
            state_url: nonempty(&self.state_url),
            timestamps,
            url: None,
            status_display_type: None,
        }
    }
}

pub fn youtube_music_search_url(query: &str, media_kind: &str) -> String {
    let query = format!("{} {}", media_kind.trim(), query.trim());
    let encoded = query
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect::<String>();
    format!("https://music.youtube.com/search?q={encoded}")
}

fn validate_button(label: &str, url: &str) -> Result<(), String> {
    let label = label.trim();
    let url = url.trim();

    if label.is_empty() != url.is_empty() {
        return Err("Each RPC button needs both a label and a URL.".into());
    }
    if label.chars().count() > BUTTON_LABEL_LIMIT {
        return Err(format!(
            "RPC button labels must be {BUTTON_LABEL_LIMIT} characters or fewer."
        ));
    }
    if url.len() > BUTTON_URL_LIMIT {
        return Err(format!(
            "RPC button URLs must be {BUTTON_URL_LIMIT} characters or fewer."
        ));
    }
    if !url.is_empty() && !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("RPC button URLs must use HTTP or HTTPS.".into());
    }

    Ok(())
}

fn validate_link_url(field: &str, url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(());
    }
    if url.len() > BUTTON_URL_LIMIT {
        return Err(format!("{field} must be {BUTTON_URL_LIMIT} characters or fewer."));
    }
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("{field} must use HTTP or HTTPS."));
    }
    Ok(())
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{DetectedMedia, PresenceDraft, presence_from_detected, youtube_music_search_url};

    fn draft() -> PresenceDraft {
        PresenceDraft {
            name: "YouTube Music".into(),
            details: "Song".into(),
            state: "Artist".into(),
            activity_type: 2,
            playing: true,
            position_seconds: 30,
            duration_seconds: 180,
            large_image: String::new(),
            large_text: String::new(),
            details_url: String::new(),
            state_url: String::new(),
            large_url: String::new(),
            small_url: String::new(),
            button1_label: "Open in YouTube Music".into(),
            button1_url: "https://music.youtube.com/watch?v=abc".into(),
            button2_label: String::new(),
            button2_url: String::new(),
        }
    }

    #[test]
    fn validates_custom_activity_and_button_pairs() {
        assert!(draft().validate().is_ok());

        let mut invalid = draft();
        invalid.button2_label = "Open".into();
        assert!(invalid.validate().is_err());

        let mut invalid = draft();
        invalid.button1_url = "javascript:alert(1)".into();
        assert!(invalid.validate().is_err());

        let mut invalid = draft();
        invalid.duration_seconds = u64::MAX;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn detected_media_builds_links_and_watching_type() {
        let media = DetectedMedia {
            source: "Browser".into(),
            app: "Chrome".into(),
            title: "Never Gonna Give You Up".into(),
            artist: "Rick Astley".into(),
            album: "Whenever You Need Somebody".into(),
            duration_seconds: 212,
            position_seconds: 30,
            playing: true,
            video_url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into(),
            channel_url: "https://www.youtube.com/@RickAstley".into(),
            artwork_url: "https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg".into(),
            site: "YouTube".into(),
            watching: true,
            status: "ok".into(),
        };
        let presence = presence_from_detected(&media, "Auto", true).expect("valid presence");
        assert_eq!(presence.name, "YouTube");
        assert_eq!(presence.activity_type, 3);
        assert_eq!(presence.state, "Rick Astley · Whenever You Need Somebody");
        assert_eq!(presence.button1_label, "Open in YouTube");
        assert_eq!(presence.button2_label, "Visit channel");
        assert!(presence.button1_url.contains("dQw4w9WgXcQ"));

        // Media links are also attached to the details/state text and artwork.
        assert_eq!(presence.details_url, media.video_url);
        assert_eq!(presence.state_url, media.channel_url);
        assert_eq!(presence.large_image, media.artwork_url);
        assert_eq!(presence.large_url, media.video_url);
        let ipc = serde_json::to_value(presence.ipc_activity()).unwrap();
        assert_eq!(ipc["details_url"], "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(ipc["state_url"], "https://www.youtube.com/@RickAstley");
        assert_eq!(ipc["assets"]["large_url"], "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        let gateway = presence.gateway_activity();
        assert_eq!(gateway.details_url.as_deref(), Some(media.video_url.as_str()));
        assert_eq!(gateway.state_url.as_deref(), Some(media.channel_url.as_str()));
        assert_eq!(gateway.assets.unwrap().large_url.as_deref(), Some(media.video_url.as_str()));

        let listening = presence_from_detected(&media, "Listening", false).unwrap();
        assert_eq!(listening.activity_type, 2);
        assert_eq!(listening.state, "Rick Astley");
    }

    #[test]
    fn creates_encoded_youtube_music_search_link() {
        assert_eq!(
            youtube_music_search_url("Song & Artist", "song"),
            "https://music.youtube.com/search?q=song+Song+%26+Artist"
        );
    }

    #[test]
    fn gateway_activity_includes_link_buttons_and_playing_timeline() {
        let mut presence = draft();
        presence.button2_label = "Open album".into();
        presence.button2_url = "https://music.youtube.com/playlist?list=abc".into();

        let activity = presence.gateway_activity();
        assert_eq!(activity.buttons.len(), 2);
        let timeline = activity.timestamps.expect("playing media has a timeline");
        assert!(timeline.start.unwrap() < timeline.end.unwrap());

        presence.playing = false;
        assert!(presence.gateway_activity().timestamps.is_none());
    }
}
