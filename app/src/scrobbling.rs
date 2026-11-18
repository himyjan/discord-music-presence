use keyring::{Entry, Error as KeyringError};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use crate::MainWindow;

const KEYRING_SERVICE: &str = "app.musicpresence.discord";
const KEYRING_ACCOUNT: &str = "scrobbling-services";
const LASTFM_API: &str = "https://ws.audioscrobbler.com/2.0/";
const LIBREFM_API: &str = "https://libre.fm/2.0/";
const LISTENBRAINZ_API: &str = "https://api.listenbrainz.org/1/submit-listens";
const API_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;
const SCROBBLE_LIMIT_SECONDS: u64 = 240;
const SCROBBLE_CHECK_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provider {
    LastFm,
    ListenBrainz,
    LibreFm,
}

impl Provider {
    fn label(self) -> &'static str {
        match self {
            Self::LastFm => "Last.fm",
            Self::ListenBrainz => "ListenBrainz",
            Self::LibreFm => "Libre.fm",
        }
    }

    fn api_url(self) -> Option<&'static str> {
        match self {
            Self::LastFm => Some(LASTFM_API),
            Self::LibreFm => Some(LIBREFM_API),
            Self::ListenBrainz => None,
        }
    }

    fn authorization_url(self) -> &'static str {
        match self {
            Self::LastFm => "https://www.last.fm/api/auth/",
            Self::ListenBrainz => "https://listenbrainz.org/settings/",
            Self::LibreFm => "https://libre.fm/api/auth/",
        }
    }
}

pub struct ScrobbleTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_seconds: Option<u64>,
    pub position_seconds: u64,
    pub playing: bool,
}

#[derive(Serialize, Deserialize)]
struct LegacyCredentials {
    api_key: String,
    api_secret: String,
    session_key: String,
}

impl Drop for LegacyCredentials {
    fn drop(&mut self) {
        self.api_secret.zeroize();
        self.session_key.zeroize();
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Credentials {
    #[serde(default)]
    lastfm: Option<LegacyCredentials>,
    #[serde(default)]
    listenbrainz_token: Option<String>,
    #[serde(default)]
    librefm: Option<LegacyCredentials>,
}

enum ProviderCredentials {
    Legacy(LegacyCredentials),
    ListenBrainz(String),
}

impl Drop for Credentials {
    fn drop(&mut self) {
        if let Some(token) = &mut self.listenbrainz_token {
            token.zeroize();
        }
    }
}

struct PendingAuthorization {
    provider: Provider,
    api_key: String,
    api_secret: String,
    token: String,
}

#[derive(Deserialize)]
struct AuthorizationTokenResponse {
    token: Option<String>,
    error: Option<i64>,
}

impl Drop for AuthorizationTokenResponse {
    fn drop(&mut self) {
        if let Some(token) = &mut self.token {
            token.zeroize();
        }
    }
}

#[derive(Deserialize)]
struct AuthorizationSessionResponse {
    session: Option<AuthorizationSession>,
    error: Option<i64>,
}

#[derive(Deserialize)]
struct AuthorizationSession {
    key: String,
}

impl Drop for AuthorizationSession {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl Drop for PendingAuthorization {
    fn drop(&mut self) {
        self.api_secret.zeroize();
        self.token.zeroize();
    }
}

pub enum Command {
    Authorize {
        provider: Provider,
        api_key: String,
        api_secret: String,
    },
    CompleteAuthorization(Provider),
    SaveListenBrainzToken(String),
    Disconnect(Provider),
    SetTrack(Option<ScrobbleTrack>),
}

pub fn run_worker(commands: Receiver<Command>, ui: slint::Weak<MainWindow>) {
    let mut credentials = match load_credentials() {
        Ok(credentials) => {
            update_connected_states(&ui, &credentials);
            credentials
        }
        Err(error) => {
            set_all_statuses(
                &ui,
                &format!("Could not load scrobbling credentials: {error}"),
            );
            Credentials::default()
        }
    };

    let mut pending = None;
    let mut playback: Option<ActivePlayback> = None;
    loop {
        let should_check_timer = playback.as_ref().is_some_and(|active| {
            active.started_at.is_some()
                && scrobble_threshold(active.duration_seconds).is_some()
                && has_pending_provider(active, &credentials)
        });
        let received = if should_check_timer {
            commands.recv_timeout(SCROBBLE_CHECK_INTERVAL)
        } else {
            commands.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match received {
            Ok(Command::Authorize {
                provider,
                api_key,
                api_secret,
            }) => {
                set_provider_state(
                    &ui,
                    provider,
                    is_connected(&credentials, provider),
                    true,
                    false,
                    format!("Preparing {} authorization...", provider.label()),
                );
                match begin_authorization(provider, api_key, api_secret) {
                    Ok(authorization) => {
                        pending = Some(authorization);
                        set_provider_state(
                            &ui,
                            provider,
                            is_connected(&credentials, provider),
                            false,
                            true,
                            format!(
                                "Authorize Music Presence in your browser, then click Finish sign-in."
                            ),
                        );
                    }
                    Err(error) => set_provider_state(
                        &ui,
                        provider,
                        is_connected(&credentials, provider),
                        false,
                        false,
                        error,
                    ),
                }
            }
            Ok(Command::CompleteAuthorization(provider)) => {
                let Some(authorization) = pending.as_ref() else {
                    set_provider_state(
                        &ui,
                        provider,
                        is_connected(&credentials, provider),
                        false,
                        false,
                        format!("Start {} authorization first.", provider.label()),
                    );
                    continue;
                };
                if authorization.provider != provider {
                    set_provider_state(
                        &ui,
                        provider,
                        is_connected(&credentials, provider),
                        false,
                        false,
                        "Finish the authorization currently open in your browser first.".into(),
                    );
                    continue;
                }
                match complete_authorization(authorization) {
                    Ok(legacy) => {
                        let previous = match provider {
                            Provider::LastFm => credentials.lastfm.replace(legacy),
                            Provider::LibreFm => credentials.librefm.replace(legacy),
                            Provider::ListenBrainz => {
                                drop(legacy);
                                set_provider_state(
                                    &ui,
                                    provider,
                                    false,
                                    false,
                                    false,
                                    "ListenBrainz uses a user token instead of browser authorization.".into(),
                                );
                                continue;
                            }
                        };
                        pending = None;
                        match save_credentials(&credentials) {
                            Ok(()) => {
                                drop(previous);
                                set_provider_state(
                                    &ui,
                                    provider,
                                    true,
                                    false,
                                    false,
                                    format!("Connected to {}.", provider.label()),
                                );
                                resume_configured_scrobbling(&mut playback, &credentials, &ui);
                            }
                            Err(error) => {
                                match provider {
                                    Provider::LastFm => credentials.lastfm = previous,
                                    Provider::LibreFm => credentials.librefm = previous,
                                    Provider::ListenBrainz => {}
                                }
                                set_provider_state(
                                    &ui,
                                    provider,
                                    is_connected(&credentials, provider),
                                    false,
                                    false,
                                    format!(
                                        "Authorization succeeded, but credentials could not be saved: {error}"
                                    ),
                                );
                            }
                        }
                    }
                    Err(error) => set_provider_state(
                        &ui,
                        provider,
                        is_connected(&credentials, provider),
                        false,
                        true,
                        error,
                    ),
                }
            }
            Ok(Command::SaveListenBrainzToken(mut entered_token)) => {
                let token = entered_token.trim().to_owned();
                entered_token.zeroize();
                if token.is_empty() {
                    set_provider_state(
                        &ui,
                        Provider::ListenBrainz,
                        is_connected(&credentials, Provider::ListenBrainz),
                        false,
                        false,
                        "Enter a ListenBrainz user token.".into(),
                    );
                    continue;
                }
                let previous = credentials.listenbrainz_token.replace(token);
                match save_credentials(&credentials) {
                    Ok(()) => {
                        if let Some(mut previous) = previous {
                            previous.zeroize();
                        }
                        set_provider_state(
                            &ui,
                            Provider::ListenBrainz,
                            true,
                            false,
                            false,
                            "ListenBrainz token saved.".into(),
                        );
                        clear_listenbrainz_token_input(&ui);
                        resume_configured_scrobbling(&mut playback, &credentials, &ui);
                    }
                    Err(error) => {
                        if let Some(token) = &mut credentials.listenbrainz_token {
                            token.zeroize();
                        }
                        credentials.listenbrainz_token = previous;
                        set_provider_state(
                            &ui,
                            Provider::ListenBrainz,
                            is_connected(&credentials, Provider::ListenBrainz),
                            false,
                            false,
                            format!("Could not save ListenBrainz token: {error}"),
                        );
                    }
                }
            }
            Ok(Command::Disconnect(provider)) => {
                let previous = match provider {
                    Provider::LastFm => credentials.lastfm.take().map(ProviderCredentials::Legacy),
                    Provider::ListenBrainz => credentials
                        .listenbrainz_token
                        .take()
                        .map(ProviderCredentials::ListenBrainz),
                    Provider::LibreFm => {
                        credentials.librefm.take().map(ProviderCredentials::Legacy)
                    }
                };
                if let Err(error) = save_credentials(&credentials) {
                    restore_provider_credentials(&mut credentials, provider, previous);
                    set_provider_state(
                        &ui,
                        provider,
                        is_connected(&credentials, provider),
                        false,
                        false,
                        format!(
                            "Could not remove saved {} credentials: {error}",
                            provider.label()
                        ),
                    );
                } else {
                    drop(previous);
                    set_provider_state(
                        &ui,
                        provider,
                        false,
                        false,
                        false,
                        format!("Disconnected from {} on this device.", provider.label()),
                    );
                }
            }
            Ok(Command::SetTrack(track)) => {
                update_playback(&mut playback, track, &credentials, &ui);
            }
            Err(RecvTimeoutError::Timeout) => {
                if let Some(active) = playback.as_mut() {
                    scrobble_if_due(active, &credentials, &ui);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    if let Some(active) = playback.as_mut() {
        scrobble_if_due(active, &credentials, &ui);
        clear_listenbrainz_now_playing(&credentials, &ui);
    }
}

struct ActivePlayback {
    title: String,
    artist: String,
    album: String,
    duration_seconds: Option<u64>,
    listened_at: u64,
    played_before: Duration,
    started_at: Option<Instant>,
    scrobbled: ScrobbleAttempts,
}

#[derive(Default)]
struct ScrobbleAttempts {
    lastfm: bool,
    listenbrainz: bool,
    librefm: bool,
}

impl ActivePlayback {
    fn elapsed(&self) -> Duration {
        self.played_before
            + self
                .started_at
                .map_or(Duration::ZERO, |started| started.elapsed())
    }

    fn is_same_track(&self, track: &ScrobbleTrack) -> bool {
        self.title == track.title.trim() && self.artist == track.artist.trim()
    }

    fn is_restart(&self, track: &ScrobbleTrack) -> bool {
        let elapsed = self.elapsed().as_secs();
        self.is_same_track(track)
            && track.playing
            && elapsed >= 30
            && track.position_seconds.saturating_add(15) < elapsed
    }
}

fn update_playback(
    playback: &mut Option<ActivePlayback>,
    track: Option<ScrobbleTrack>,
    credentials: &Credentials,
    ui: &slint::Weak<MainWindow>,
) {
    let Some(track) = track else {
        if let Some(active) = playback.as_mut() {
            scrobble_if_due(active, credentials, ui);
        }
        clear_listenbrainz_now_playing(credentials, ui);
        *playback = None;
        return;
    };

    let title = track.title.trim();
    let artist = track.artist.trim();
    if title.is_empty() || artist.is_empty() {
        for provider in [Provider::LastFm, Provider::ListenBrainz, Provider::LibreFm] {
            set_provider_state(
                ui,
                provider,
                is_connected(credentials, provider),
                false,
                false,
                "Scrobbling needs both a track title and artist.".into(),
            );
        }
        *playback = None;
        return;
    }

    if let Some(active) = playback.as_mut()
        && active.is_same_track(&track)
        && !active.is_restart(&track)
    {
        let was_playing = active.started_at.is_some();
        let elapsed = active.elapsed();
        active.played_before = elapsed;
        active.started_at = track.playing.then(Instant::now);
        active.duration_seconds = track.duration_seconds;
        active.album = track.album.trim().to_owned();
        if was_playing && !track.playing {
            clear_listenbrainz_now_playing(credentials, ui);
        } else if !was_playing && track.playing {
            notify_now_playing(active, credentials, ui);
        }
        scrobble_if_due(active, credentials, ui);
        return;
    }

    if let Some(active) = playback.as_mut() {
        scrobble_if_due(active, credentials, ui);
    }
    if !track.playing {
        clear_listenbrainz_now_playing(credentials, ui);
    }

    let listened_at = unix_seconds().saturating_sub(track.position_seconds);
    let mut active = ActivePlayback {
        title: title.to_owned(),
        artist: artist.to_owned(),
        album: track.album.trim().to_owned(),
        duration_seconds: track.duration_seconds,
        listened_at,
        played_before: Duration::from_secs(track.position_seconds),
        started_at: track.playing.then(Instant::now),
        scrobbled: ScrobbleAttempts::default(),
    };
    if track.playing {
        notify_now_playing(&active, credentials, ui);
    }
    scrobble_if_due(&mut active, credentials, ui);
    *playback = Some(active);
}

fn scrobble_if_due(
    playback: &mut ActivePlayback,
    credentials: &Credentials,
    ui: &slint::Weak<MainWindow>,
) {
    if !should_scrobble(playback) {
        return;
    }
    submit_scrobble(playback, credentials, ui);
}

fn should_scrobble(playback: &ActivePlayback) -> bool {
    scrobble_threshold(playback.duration_seconds)
        .is_some_and(|threshold| playback.elapsed() >= threshold)
}

fn has_pending_provider(playback: &ActivePlayback, credentials: &Credentials) -> bool {
    (credentials.lastfm.is_some() && !playback.scrobbled.lastfm)
        || (credentials.listenbrainz_token.is_some() && !playback.scrobbled.listenbrainz)
        || (credentials.librefm.is_some() && !playback.scrobbled.librefm)
}

fn resume_configured_scrobbling(
    playback: &mut Option<ActivePlayback>,
    credentials: &Credentials,
    ui: &slint::Weak<MainWindow>,
) {
    if let Some(active) = playback.as_mut() {
        if active.started_at.is_some() {
            notify_now_playing(active, credentials, ui);
        }
        scrobble_if_due(active, credentials, ui);
    }
}

fn scrobble_threshold(duration_seconds: Option<u64>) -> Option<Duration> {
    match duration_seconds {
        Some(duration) if duration < 30 => None,
        Some(duration) => Some(Duration::from_secs(
            (duration / 2).min(SCROBBLE_LIMIT_SECONDS),
        )),
        None => Some(Duration::from_secs(SCROBBLE_LIMIT_SECONDS)),
    }
}

fn notify_now_playing(
    playback: &ActivePlayback,
    credentials: &Credentials,
    ui: &slint::Weak<MainWindow>,
) {
    if let Some(legacy) = &credentials.lastfm {
        report_result(
            ui,
            Provider::LastFm,
            legacy_now_playing(Provider::LastFm, legacy, playback),
        );
    }
    if let Some(token) = &credentials.listenbrainz_token {
        report_result(
            ui,
            Provider::ListenBrainz,
            listenbrainz_submit(token, playback, true),
        );
    }
    if let Some(legacy) = &credentials.librefm {
        report_result(
            ui,
            Provider::LibreFm,
            legacy_now_playing(Provider::LibreFm, legacy, playback),
        );
    }
}

fn submit_scrobble(
    playback: &mut ActivePlayback,
    credentials: &Credentials,
    ui: &slint::Weak<MainWindow>,
) {
    if let Some(legacy) = &credentials.lastfm
        && !playback.scrobbled.lastfm
    {
        playback.scrobbled.lastfm = true;
        report_result(
            ui,
            Provider::LastFm,
            legacy_scrobble(Provider::LastFm, legacy, playback),
        );
    }
    if let Some(token) = &credentials.listenbrainz_token
        && !playback.scrobbled.listenbrainz
    {
        playback.scrobbled.listenbrainz = true;
        report_result(
            ui,
            Provider::ListenBrainz,
            listenbrainz_submit(token, playback, false),
        );
    }
    if let Some(legacy) = &credentials.librefm
        && !playback.scrobbled.librefm
    {
        playback.scrobbled.librefm = true;
        report_result(
            ui,
            Provider::LibreFm,
            legacy_scrobble(Provider::LibreFm, legacy, playback),
        );
    }
}

fn report_result(ui: &slint::Weak<MainWindow>, provider: Provider, result: Result<(), String>) {
    let connected = true;
    let status = match result {
        Ok(()) => format!("{} activity updated.", provider.label()),
        Err(error) => format!("{} error: {error}", provider.label()),
    };
    set_provider_state(ui, provider, connected, false, false, status);
}

fn legacy_now_playing(
    provider: Provider,
    credentials: &LegacyCredentials,
    playback: &ActivePlayback,
) -> Result<(), String> {
    let mut fields = BTreeMap::from([
        ("artist".to_owned(), playback.artist.clone()),
        ("track".to_owned(), playback.title.clone()),
    ]);
    if !playback.album.is_empty() {
        fields.insert("album".into(), playback.album.clone());
    }
    if let Some(duration) = playback.duration_seconds {
        fields.insert("duration".into(), duration.to_string());
    }
    legacy_call(provider, credentials, "track.updateNowPlaying", fields)?;
    Ok(())
}

fn legacy_scrobble(
    provider: Provider,
    credentials: &LegacyCredentials,
    playback: &ActivePlayback,
) -> Result<(), String> {
    let mut fields = BTreeMap::from([
        ("artist".to_owned(), playback.artist.clone()),
        ("timestamp".to_owned(), playback.listened_at.to_string()),
        ("track".to_owned(), playback.title.clone()),
    ]);
    if !playback.album.is_empty() {
        fields.insert("album".into(), playback.album.clone());
    }
    if let Some(duration) = playback.duration_seconds {
        fields.insert("duration".into(), duration.to_string());
    }
    legacy_call(provider, credentials, "track.scrobble", fields)?;
    Ok(())
}

fn legacy_call(
    provider: Provider,
    credentials: &LegacyCredentials,
    method: &str,
    mut fields: BTreeMap<String, String>,
) -> Result<Value, String> {
    let endpoint = provider.api_url().ok_or_else(|| {
        format!(
            "{} does not use the Last.fm-compatible API.",
            provider.label()
        )
    })?;
    fields.insert("api_key".into(), credentials.api_key.clone());
    fields.insert("method".into(), method.into());
    fields.insert("sk".into(), credentials.session_key.clone());
    let body = signed_form_response(provider, endpoint, fields, &credentials.api_secret)?;
    let value: Value = serde_json::from_str(&body)
        .map_err(|_| format!("{} returned an invalid API response.", provider.label()))?;
    if let Some(error) = value.get("error") {
        let code = error.as_i64().unwrap_or_default();
        return Err(format!(
            "{} rejected the request (API error {code}).",
            provider.label()
        ));
    }
    Ok(value)
}

fn signed_form_response(
    provider: Provider,
    endpoint: &str,
    mut fields: BTreeMap<String, String>,
    api_secret: &str,
) -> Result<Zeroizing<String>, String> {
    let result = (|| {
        let signature = api_signature(&fields, api_secret);
        fields.insert("api_sig".into(), signature);
        fields.insert("format".into(), "json".into());
        let form = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>();
        let response = agent()
            .post(endpoint)
            .set("Accept", "application/json")
            .send_form(&form)
            .map_err(|error| format!("{} request failed: {error}", provider.label()))?;
        read_response(response)
    })();
    for value in fields.values_mut() {
        value.zeroize();
    }
    result
}

fn signed_get_response(
    provider: Provider,
    endpoint: &str,
    mut fields: BTreeMap<String, String>,
    api_secret: &str,
) -> Result<Zeroizing<String>, String> {
    let result = (|| {
        let signature = api_signature(&fields, api_secret);
        fields.insert("api_sig".into(), signature);
        fields.insert("format".into(), "json".into());
        let mut url = Url::parse(endpoint)
            .map_err(|error| format!("Invalid {} API URL: {error}", provider.label()))?;
        url.query_pairs_mut()
            .extend_pairs(fields.iter().map(|(key, value)| (key, value)));
        let response = agent()
            .get(url.as_str())
            .set("Accept", "application/json")
            .call()
            .map_err(|error| format!("{} request failed: {error}", provider.label()))?;
        read_response(response)
    })();
    for value in fields.values_mut() {
        value.zeroize();
    }
    result
}

fn api_signature(fields: &BTreeMap<String, String>, secret: &str) -> String {
    let mut digest = Md5::new();
    for (key, value) in fields {
        digest.update(key.as_bytes());
        digest.update(value.as_bytes());
    }
    digest.update(secret.as_bytes());
    format!("{:x}", digest.finalize())
}

fn listenbrainz_submit(
    token: &str,
    playback: &ActivePlayback,
    playing_now: bool,
) -> Result<(), String> {
    let payload = listenbrainz_payload(playback, playing_now);
    let body = serde_json::to_string(&payload)
        .map_err(|error| format!("Could not encode ListenBrainz listen: {error}"))?;
    let response = agent()
        .post(LISTENBRAINZ_API)
        .set("Authorization", &format!("Token {token}"))
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .send_string(&body)
        .map_err(|error| format!("ListenBrainz submission failed: {error}"))?;
    let _ = read_response(response)?;
    Ok(())
}

fn clear_listenbrainz_now_playing(credentials: &Credentials, ui: &slint::Weak<MainWindow>) {
    if let Some(token) = &credentials.listenbrainz_token {
        let body = r#"{"client":"Music Presence"}"#;
        let result = agent()
            .post("https://api.listenbrainz.org/1/playing-now/delete")
            .set("Authorization", &format!("Token {token}"))
            .set("Content-Type", "application/json")
            .send_string(body)
            .map_err(|error| format!("Could not clear ListenBrainz now-playing status: {error}"))
            .and_then(|response| read_response(response).map(|_| ()));
        report_result(ui, Provider::ListenBrainz, result);
    }
}

fn listenbrainz_payload(playback: &ActivePlayback, playing_now: bool) -> Value {
    let mut metadata = json!({
        "artist_name": playback.artist,
        "track_name": playback.title,
        "additional_info": {
            "submission_client": "Music Presence",
            "submission_client_version": env!("CARGO_PKG_VERSION")
        }
    });
    if !playback.album.is_empty() {
        metadata["release_name"] = json!(playback.album);
    }
    if let Some(duration) = playback.duration_seconds {
        metadata["additional_info"]["duration_ms"] = json!(duration.saturating_mul(1000));
    }
    let mut listen = json!({ "track_metadata": metadata });
    if !playing_now {
        listen["listened_at"] = json!(playback.listened_at);
    }
    json!({
        "listen_type": if playing_now { "playing_now" } else { "single" },
        "payload": [listen]
    })
}

fn begin_authorization(
    provider: Provider,
    api_key: String,
    api_secret: String,
) -> Result<PendingAuthorization, String> {
    let api_key = api_key.trim().to_owned();
    let api_secret = Zeroizing::new(api_secret.trim().to_owned());
    if api_key.is_empty() || api_secret.is_empty() {
        return Err(format!(
            "Enter the {} API key and API secret first.",
            provider.label()
        ));
    }
    if provider.api_url().is_none() {
        return Err("ListenBrainz uses a user token instead of browser authorization.".into());
    }

    let endpoint = provider.api_url().ok_or_else(|| {
        "ListenBrainz uses a user token instead of browser authorization.".to_string()
    })?;
    let fields = BTreeMap::from([
        ("api_key".to_owned(), api_key.clone()),
        ("method".to_owned(), "auth.getToken".into()),
    ]);
    let body = signed_get_response(provider, endpoint, fields, &api_secret).map_err(|error| {
        format!(
            "Could not request {} authorization: {error}",
            provider.label()
        )
    })?;
    let response: AuthorizationTokenResponse = serde_json::from_str(&body).map_err(|_| {
        format!(
            "{} returned an invalid authorization response.",
            provider.label()
        )
    })?;
    if response.error.is_some() {
        return Err(format!(
            "{} rejected the authorization request. Check the API key and secret.",
            provider.label()
        ));
    }
    let token = response
        .token
        .as_deref()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            format!(
                "{} did not return an authorization token.",
                provider.label()
            )
        })?
        .to_owned();
    let mut authorization_url = Url::parse(provider.authorization_url())
        .map_err(|error| format!("Invalid {} authorization URL: {error}", provider.label()))?;
    authorization_url
        .query_pairs_mut()
        .append_pair("api_key", &api_key)
        .append_pair("token", &token);
    webbrowser::open(authorization_url.as_str()).map_err(|error| {
        format!(
            "Could not open the {} authorization page: {error}",
            provider.label()
        )
    })?;

    Ok(PendingAuthorization {
        provider,
        api_key,
        api_secret: api_secret.to_string(),
        token,
    })
}

fn complete_authorization(
    authorization: &PendingAuthorization,
) -> Result<LegacyCredentials, String> {
    let fields = BTreeMap::from([
        ("api_key".to_owned(), authorization.api_key.clone()),
        ("method".to_owned(), "auth.getSession".into()),
        ("token".to_owned(), authorization.token.clone()),
    ]);
    let endpoint = authorization
        .provider
        .api_url()
        .ok_or_else(|| "ListenBrainz does not use this authorization flow.".to_string())?;
    let body = signed_get_response(
        authorization.provider,
        endpoint,
        fields,
        &authorization.api_secret,
    )
    .map_err(|error| {
        format!(
            "Could not finish {} authorization: {error}",
            authorization.provider.label()
        )
    })?;
    let response: AuthorizationSessionResponse = serde_json::from_str(&body).map_err(|_| {
        format!(
            "{} returned an invalid session response.",
            authorization.provider.label()
        )
    })?;
    if response.error.is_some() {
        return Err(format!(
            "{} authorization is not complete. Approve access in the browser, then try again.",
            authorization.provider.label()
        ));
    }
    let session_key = response
        .session
        .as_ref()
        .map(|session| session.key.as_str())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            format!(
                "{} did not return a user session.",
                authorization.provider.label()
            )
        })?;
    Ok(LegacyCredentials {
        api_key: authorization.api_key.clone(),
        api_secret: authorization.api_secret.clone(),
        session_key: session_key.to_owned(),
    })
}

fn read_response(response: ureq::Response) -> Result<Zeroizing<String>, String> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Could not read scrobbling service response: {error}"))?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err("Scrobbling service response exceeded the size limit.".into());
    }
    String::from_utf8(bytes)
        .map(Zeroizing::new)
        .map_err(|_| "Scrobbling service returned a non-UTF-8 response.".into())
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(API_TIMEOUT)
        .timeout_read(API_TIMEOUT)
        .timeout_write(API_TIMEOUT)
        .build()
}

fn load_credentials() -> Result<Credentials, String> {
    let entry = credential_entry()?;
    let bytes = Zeroizing::new(match entry.get_secret() {
        Ok(bytes) => bytes,
        Err(KeyringError::NoEntry) => return Ok(Credentials::default()),
        Err(error) => return Err(format!("Could not read scrobbling credentials: {error}")),
    });
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Saved scrobbling credentials are invalid: {error}"))
}

fn save_credentials(credentials: &Credentials) -> Result<(), String> {
    let entry = credential_entry()?;
    if credentials.lastfm.is_none()
        && credentials.listenbrainz_token.is_none()
        && credentials.librefm.is_none()
    {
        return match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(error) => Err(format!("Could not remove scrobbling credentials: {error}")),
        };
    }
    let serialized = Zeroizing::new(
        serde_json::to_vec(credentials)
            .map_err(|error| format!("Could not encode scrobbling credentials: {error}"))?,
    );
    entry
        .set_secret(&serialized)
        .map_err(|error| format!("Could not save scrobbling credentials: {error}"))
}

fn credential_entry() -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|error| format!("OS credential store is unavailable: {error}"))
}

fn is_connected(credentials: &Credentials, provider: Provider) -> bool {
    match provider {
        Provider::LastFm => credentials.lastfm.is_some(),
        Provider::ListenBrainz => credentials.listenbrainz_token.is_some(),
        Provider::LibreFm => credentials.librefm.is_some(),
    }
}

fn restore_provider_credentials(
    credentials: &mut Credentials,
    provider: Provider,
    previous: Option<ProviderCredentials>,
) {
    match (provider, previous) {
        (Provider::LastFm, Some(ProviderCredentials::Legacy(legacy))) => {
            credentials.lastfm = Some(legacy);
        }
        (Provider::ListenBrainz, Some(ProviderCredentials::ListenBrainz(token))) => {
            credentials.listenbrainz_token = Some(token);
        }
        (Provider::LibreFm, Some(ProviderCredentials::Legacy(legacy))) => {
            credentials.librefm = Some(legacy);
        }
        (_, Some(other)) => clear_provider_credentials(Some(other)),
        (_, None) => {}
    }
}

fn clear_provider_credentials(mut credentials: Option<ProviderCredentials>) {
    if let Some(ProviderCredentials::ListenBrainz(token)) = &mut credentials {
        token.zeroize();
    }
}

fn update_connected_states(ui: &slint::Weak<MainWindow>, credentials: &Credentials) {
    for provider in [Provider::LastFm, Provider::ListenBrainz, Provider::LibreFm] {
        let connected = is_connected(credentials, provider);
        set_provider_state(
            ui,
            provider,
            connected,
            false,
            false,
            if connected {
                format!("Connected to {}.", provider.label())
            } else {
                "Not connected.".into()
            },
        );
    }
}

fn set_all_statuses(ui: &slint::Weak<MainWindow>, status: &str) {
    for provider in [Provider::LastFm, Provider::ListenBrainz, Provider::LibreFm] {
        set_provider_state(ui, provider, false, false, false, status.to_owned());
    }
}

fn set_provider_state(
    ui: &slint::Weak<MainWindow>,
    provider: Provider,
    connected: bool,
    busy: bool,
    pending: bool,
    status: String,
) {
    let ui = ui.clone();
    if let Err(error) = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            match provider {
                Provider::LastFm => {
                    window.set_scrobble_lastfm_connected(connected);
                    window.set_scrobble_lastfm_busy(busy);
                    window.set_scrobble_lastfm_pending(pending);
                    window.set_scrobble_lastfm_status(status.into());
                    if connected {
                        window.set_scrobble_lastfm_api_secret("".into());
                    }
                }
                Provider::ListenBrainz => {
                    window.set_scrobble_listenbrainz_connected(connected);
                    window.set_scrobble_listenbrainz_busy(busy);
                    window.set_scrobble_listenbrainz_status(status.into());
                }
                Provider::LibreFm => {
                    window.set_scrobble_librefm_connected(connected);
                    window.set_scrobble_librefm_busy(busy);
                    window.set_scrobble_librefm_pending(pending);
                    window.set_scrobble_librefm_status(status.into());
                    if connected {
                        window.set_scrobble_librefm_api_secret("".into());
                    }
                }
            }
        }
    }) {
        eprintln!(
            "Could not update {} scrobbling status: {error}",
            provider.label()
        );
    }
}

fn clear_listenbrainz_token_input(ui: &slint::Weak<MainWindow>) {
    let ui = ui.clone();
    if let Err(error) = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            window.set_scrobble_listenbrainz_token("".into());
        }
    }) {
        eprintln!("Could not clear ListenBrainz token field: {error}");
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::{
        ActivePlayback, Credentials, LegacyCredentials, Provider, ScrobbleTrack, api_signature,
        has_pending_provider, listenbrainz_payload, scrobble_threshold, should_scrobble,
    };
    use std::{collections::BTreeMap, time::Duration};

    fn playback() -> ActivePlayback {
        ActivePlayback {
            title: "Test Track".into(),
            artist: "Test Artist".into(),
            album: "Test Album".into(),
            duration_seconds: Some(200),
            listened_at: 1_700_000_000,
            played_before: Duration::ZERO,
            started_at: None,
            scrobbled: Default::default(),
        }
    }

    #[test]
    fn scrobble_threshold_is_half_track_or_four_minutes() {
        assert_eq!(
            scrobble_threshold(Some(200)),
            Some(Duration::from_secs(100))
        );
        assert_eq!(
            scrobble_threshold(Some(900)),
            Some(Duration::from_secs(240))
        );
        assert_eq!(scrobble_threshold(None), Some(Duration::from_secs(240)));
        assert_eq!(scrobble_threshold(Some(29)), None);
    }

    #[test]
    fn signs_legacy_api_calls_independent_of_parameter_order() {
        let first = BTreeMap::from([
            ("api_key".to_owned(), "key".to_owned()),
            ("method".to_owned(), "auth.getToken".to_owned()),
        ]);
        let second = BTreeMap::from([
            ("method".to_owned(), "auth.getToken".to_owned()),
            ("api_key".to_owned(), "key".to_owned()),
        ]);
        assert_eq!(
            api_signature(&first, "secret"),
            api_signature(&second, "secret")
        );
        assert_eq!(api_signature(&first, "secret").len(), 32);
        assert_eq!(
            api_signature(&BTreeMap::new(), "secret"),
            "5ebe2294ecd0e0f08eab7690d2a6ee69"
        );
    }

    #[test]
    fn listen_is_due_at_threshold_and_new_services_can_receive_it() {
        let mut active = playback();
        let mut credentials = Credentials::default();
        credentials.lastfm = Some(LegacyCredentials {
            api_key: "key".into(),
            api_secret: "secret".into(),
            session_key: "session".into(),
        });
        active.played_before = Duration::from_secs(99);
        assert!(!should_scrobble(&active));
        active.played_before = Duration::from_secs(100);
        assert!(should_scrobble(&active));
        active.scrobbled.lastfm = true;
        assert!(!has_pending_provider(&active, &credentials));
        credentials.listenbrainz_token = Some("token".into());
        assert!(has_pending_provider(&active, &credentials));
    }

    #[test]
    fn detects_a_new_play_of_the_same_track_after_position_resets() {
        let mut active = playback();
        active.played_before = Duration::from_secs(100);
        let track = ScrobbleTrack {
            title: "Test Track".into(),
            artist: "Test Artist".into(),
            album: "Test Album".into(),
            duration_seconds: Some(200),
            position_seconds: 0,
            playing: true,
        };
        assert!(active.is_restart(&track));
    }

    #[test]
    fn listenbrainz_payload_separates_now_playing_from_scrobbles() {
        let playback = playback();
        let now_playing = listenbrainz_payload(&playback, true);
        let scrobble = listenbrainz_payload(&playback, false);
        assert_eq!(now_playing["listen_type"], "playing_now");
        assert!(now_playing["payload"][0].get("listened_at").is_none());
        assert_eq!(scrobble["listen_type"], "single");
        assert_eq!(scrobble["payload"][0]["listened_at"], 1_700_000_000);
    }

    #[test]
    fn only_legacy_providers_have_an_api_endpoint() {
        assert!(Provider::LastFm.api_url().is_some());
        assert!(Provider::LibreFm.api_url().is_some());
        assert!(Provider::ListenBrainz.api_url().is_none());
    }
}
