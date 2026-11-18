use std::time::{SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{self, Activity, Assets, Button, Timestamps};
use twilight_model::gateway::presence::activity_button::ActivityButtonLink;
use twilight_model::gateway::presence::{
    Activity as GatewayActivity, ActivityAssets, ActivityButton, ActivityTimestamps, ActivityType,
};

const TEXT_LIMIT: usize = 128;
const BUTTON_LABEL_LIMIT: usize = 32;
const BUTTON_URL_LIMIT: usize = 512;

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
        if !self.state.trim().is_empty() {
            activity = activity.state(self.state.trim());
        }
        if !self.large_image.trim().is_empty() {
            activity = activity.assets(
                Assets::new()
                    .large_image(self.large_image.trim())
                    .large_text(self.large_text.trim()),
            );
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
            large_url: None,
            small_image: None,
            small_text: None,
            small_url: None,
            invite_cover_image: None,
        });

        GatewayActivity {
            application_id: None,
            assets,
            buttons,
            created_at: None,
            details: nonempty(&self.details),
            details_url: None,
            emoji: None,
            flags: None,
            id: None,
            instance: None,
            kind: ActivityType::from(self.activity_type),
            name: self.name.trim().to_owned(),
            party: None,
            secrets: None,
            state: nonempty(&self.state),
            state_url: None,
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
    use super::{PresenceDraft, youtube_music_search_url};

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
