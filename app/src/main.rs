mod bot_gateway;
mod discord_rpc;
mod media_detector;
mod oauth;
mod oauth_protocol;
mod presence;
mod scrobbling;
mod token_store;
mod urls;

use bot_gateway::Command as GatewayCommand;
use discord_rpc::Command as RpcCommand;
use oauth::Command as OAuthCommand;
use presence::{PresenceDraft, youtube_music_search_url};
use scrobbling::{Command as ScrobbleCommand, Provider as ScrobbleProvider, ScrobbleTrack};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(callback_uri) = oauth_protocol::callback_argument() {
        oauth_protocol::forward_callback(&callback_uri).map_err(std::io::Error::other)?;
        return Ok(());
    }

    let window = MainWindow::new()?;
    if let Err(error) = restore_saved_token(&window) {
        window.set_gateway_status(error.into());
    }

    let (rpc_tx, rpc_rx) = mpsc::channel();
    let rpc_ui = window.as_weak();
    let rpc_worker = thread::Builder::new()
        .name("discord-rich-presence".into())
        .spawn(move || discord_rpc::run_worker(rpc_rx, rpc_ui))?;

    let (gateway_tx, gateway_rx) = mpsc::channel();
    let gateway_ui = window.as_weak();
    let gateway_worker = thread::Builder::new()
        .name("discord-bot-gateway".into())
        .spawn(move || bot_gateway::run_worker(gateway_rx, gateway_ui))?;

    let (oauth_tx, oauth_rx) = mpsc::channel();
    let oauth_ui = window.as_weak();
    let oauth_stopping = Arc::new(AtomicBool::new(false));
    let oauth_worker_stopping = oauth_stopping.clone();
    let oauth_cancel_login = Arc::new(AtomicBool::new(false));
    let oauth_worker_cancel_login = oauth_cancel_login.clone();
    let oauth_worker = thread::Builder::new()
        .name("discord-oauth2".into())
        .spawn(move || {
            oauth::run_worker(
                oauth_rx,
                oauth_ui,
                oauth_worker_stopping,
                oauth_worker_cancel_login,
            )
        })?;

    let (scrobble_tx, scrobble_rx) = mpsc::channel();
    let scrobble_ui = window.as_weak();
    let scrobble_worker = thread::Builder::new()
        .name("music-scrobbling".into())
        .spawn(move || scrobbling::run_worker(scrobble_rx, scrobble_ui))?;

    let (detector_tx, detector_rx) = mpsc::channel();
    let detector_ui = window.as_weak();
    let detector_rpc_tx = rpc_tx.clone();
    let detector_gateway_tx = gateway_tx.clone();
    let detector_scrobble_tx = scrobble_tx.clone();
    let detector_worker = thread::Builder::new()
        .name("media-detector".into())
        .spawn(move || {
            media_detector::run_worker(
                detector_rx,
                detector_ui,
                detector_rpc_tx,
                detector_gateway_tx,
                detector_scrobble_tx,
            )
        })?;

    let connect_tx = rpc_tx.clone();
    let connect_scrobble_tx = scrobble_tx.clone();
    let connect_ui = window.as_weak();
    window.on_connect_requested(move || {
        let Some(window) = connect_ui.upgrade() else {
            return;
        };
        match presence_from_ui(&window) {
            Ok(presence) => {
                if let Err(error) = connect_tx.send(RpcCommand::Connect {
                    application_id: window.get_application_id().to_string(),
                    presence,
                }) {
                    window.set_connected(false);
                    window.set_connecting(false);
                    window.set_connection_status(format!("Could not connect: {error}").into());
                } else {
                    set_scrobble_track(&connect_scrobble_tx, &window);
                }
            }
            Err(error) => {
                window.set_connecting(false);
                window.set_connection_status(error.into());
            }
        }
    });

    let disconnect_tx = rpc_tx.clone();
    let disconnect_ui = window.as_weak();
    window.on_disconnect_requested(move || {
        if let Err(error) = disconnect_tx.send(RpcCommand::Disconnect)
            && let Some(window) = disconnect_ui.upgrade()
        {
            window.set_connecting(false);
            window.set_connection_status(format!("Could not disconnect: {error}").into());
        }
    });

    let update_rpc_tx = rpc_tx.clone();
    let update_gateway_tx = gateway_tx.clone();
    let update_scrobble_tx = scrobble_tx.clone();
    let update_ui = window.as_weak();
    window.on_presence_update_requested(move || {
        let Some(window) = update_ui.upgrade() else {
            return;
        };
        match presence_from_ui(&window) {
            Ok(presence) => {
                let rpc_result = update_rpc_tx.send(RpcCommand::Update(presence.clone()));
                let gateway_result =
                    update_gateway_tx.send(GatewayCommand::UpdatePresence(presence));
                if let Err(error) = rpc_result {
                    window.set_connection_status(
                        format!("Could not update desktop RPC: {error}").into(),
                    );
                }
                if let Err(error) = gateway_result {
                    window.set_gateway_status(
                        format!("Could not update bot activity: {error}").into(),
                    );
                }
                set_scrobble_track(&update_scrobble_tx, &window);
            }
            Err(error) => window.set_connection_status(error.into()),
        }
    });

    let link_ui = window.as_weak();
    window.on_generate_youtube_music_link(move || {
        if let Some(window) = link_ui.upgrade() {
            let title = window.get_media_title().to_string();
            let artist = window.get_media_artist().to_string();
            let album = window.get_media_album().to_string();
            let query = [title.as_str(), artist.as_str(), album.as_str()]
                .into_iter()
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let url = youtube_music_search_url(&query, &window.get_media_kind());
            window.set_button1_url(url.into());
            if window.get_button1_label().trim().is_empty() {
                window.set_button1_label("Open in YouTube Music".into());
            }
        }
    });

    let login_tx = gateway_tx.clone();
    let login_scrobble_tx = scrobble_tx.clone();
    let login_ui = window.as_weak();
    window.on_gateway_login_requested(move || {
        let Some(window) = login_ui.upgrade() else {
            return;
        };
        match presence_from_ui(&window) {
            Ok(presence) => {
                let command = GatewayCommand::Login {
                    token: window.get_gateway_token().to_string(),
                    remember: window.get_remember_gateway_token(),
                    presence,
                };
                if let Err(error) = login_tx.send(command) {
                    window.set_gateway_connecting(false);
                    window.set_gateway_status(format!("Could not start bot login: {error}").into());
                } else {
                    set_scrobble_track(&login_scrobble_tx, &window);
                }
            }
            Err(error) => {
                window.set_gateway_connecting(false);
                window.set_gateway_status(error.into());
            }
        }
    });

    let logout_tx = gateway_tx.clone();
    let logout_ui = window.as_weak();
    window.on_gateway_logout_requested(move || {
        if let Err(error) = logout_tx.send(GatewayCommand::Logout)
            && let Some(window) = logout_ui.upgrade()
        {
            window.set_gateway_connecting(false);
            window.set_gateway_status(format!("Could not log out: {error}").into());
        }
    });

    let oauth_login_tx = oauth_tx.clone();
    let oauth_login_ui = window.as_weak();
    let oauth_cancel_login_on_start = oauth_cancel_login.clone();
    window.on_oauth_login_requested(move || {
        let Some(window) = oauth_login_ui.upgrade() else {
            return;
        };
        oauth_cancel_login_on_start.store(false, Ordering::Relaxed);
        if let Err(error) =
            oauth_login_tx.send(OAuthCommand::Login(window.get_application_id().to_string()))
        {
            window.set_oauth_connecting(false);
            window
                .set_oauth_status(format!("Could not start Discord OAuth2 login: {error}").into());
        }
    });

    let oauth_logout_tx = oauth_tx.clone();
    let oauth_logout_ui = window.as_weak();
    window.on_oauth_logout_requested(move || {
        if let Err(error) = oauth_logout_tx.send(OAuthCommand::Logout)
            && let Some(window) = oauth_logout_ui.upgrade()
        {
            window.set_oauth_connecting(false);
            window.set_oauth_status(format!("Could not log out of Discord: {error}").into());
        }
    });

    let cancel_login_ui = window.as_weak();
    window.on_oauth_cancel_requested(move || {
        oauth_cancel_login.store(true, Ordering::Relaxed);
        if let Some(window) = cancel_login_ui.upgrade() {
            window.set_oauth_status("Canceling browser sign-in...".into());
        }
    });

    let open_oauth_link_ui = window.as_weak();
    window.on_oauth_open_authorize_url_requested(move || {
        let Some(window) = open_oauth_link_ui.upgrade() else {
            return;
        };
        let authorize_url = window.get_oauth_authorize_url().to_string();
        match oauth::open_authorization_url(&authorize_url) {
            Ok(()) => {
                window.set_oauth_status("Opened the Discord sign-in link in your browser.".into())
            }
            Err(error) => window.set_oauth_status(
                format!("Could not open the browser: {error}. Copy the link above into a browser.")
                    .into(),
            ),
        }
    });

    let scrobble_lastfm_authorize_tx = scrobble_tx.clone();
    let scrobble_lastfm_authorize_ui = window.as_weak();
    window.on_scrobble_lastfm_authorize_requested(move || {
        let Some(window) = scrobble_lastfm_authorize_ui.upgrade() else {
            return;
        };
        send_scrobble_command(
            &scrobble_lastfm_authorize_tx,
            &window,
            ScrobbleCommand::Authorize {
                provider: ScrobbleProvider::LastFm,
                api_key: window.get_scrobble_lastfm_api_key().to_string(),
                api_secret: window.get_scrobble_lastfm_api_secret().to_string(),
            },
            ScrobbleProvider::LastFm,
        );
    });

    let scrobble_lastfm_finish_tx = scrobble_tx.clone();
    let scrobble_lastfm_finish_ui = window.as_weak();
    window.on_scrobble_lastfm_finish_requested(move || {
        if let Some(window) = scrobble_lastfm_finish_ui.upgrade() {
            send_scrobble_command(
                &scrobble_lastfm_finish_tx,
                &window,
                ScrobbleCommand::CompleteAuthorization(ScrobbleProvider::LastFm),
                ScrobbleProvider::LastFm,
            );
        }
    });

    let scrobble_lastfm_disconnect_tx = scrobble_tx.clone();
    let scrobble_lastfm_disconnect_ui = window.as_weak();
    window.on_scrobble_lastfm_disconnect_requested(move || {
        if let Some(window) = scrobble_lastfm_disconnect_ui.upgrade() {
            send_scrobble_command(
                &scrobble_lastfm_disconnect_tx,
                &window,
                ScrobbleCommand::Disconnect(ScrobbleProvider::LastFm),
                ScrobbleProvider::LastFm,
            );
        }
    });

    let scrobble_listenbrainz_save_tx = scrobble_tx.clone();
    let scrobble_listenbrainz_save_ui = window.as_weak();
    window.on_scrobble_listenbrainz_save_requested(move || {
        let Some(window) = scrobble_listenbrainz_save_ui.upgrade() else {
            return;
        };
        send_scrobble_command(
            &scrobble_listenbrainz_save_tx,
            &window,
            ScrobbleCommand::SaveListenBrainzToken(
                window.get_scrobble_listenbrainz_token().to_string(),
            ),
            ScrobbleProvider::ListenBrainz,
        );
    });

    let scrobble_listenbrainz_disconnect_tx = scrobble_tx.clone();
    let scrobble_listenbrainz_disconnect_ui = window.as_weak();
    window.on_scrobble_listenbrainz_disconnect_requested(move || {
        if let Some(window) = scrobble_listenbrainz_disconnect_ui.upgrade() {
            send_scrobble_command(
                &scrobble_listenbrainz_disconnect_tx,
                &window,
                ScrobbleCommand::Disconnect(ScrobbleProvider::ListenBrainz),
                ScrobbleProvider::ListenBrainz,
            );
        }
    });

    let scrobble_librefm_authorize_tx = scrobble_tx.clone();
    let scrobble_librefm_authorize_ui = window.as_weak();
    window.on_scrobble_librefm_authorize_requested(move || {
        let Some(window) = scrobble_librefm_authorize_ui.upgrade() else {
            return;
        };
        send_scrobble_command(
            &scrobble_librefm_authorize_tx,
            &window,
            ScrobbleCommand::Authorize {
                provider: ScrobbleProvider::LibreFm,
                api_key: window.get_scrobble_librefm_api_key().to_string(),
                api_secret: window.get_scrobble_librefm_api_secret().to_string(),
            },
            ScrobbleProvider::LibreFm,
        );
    });

    let scrobble_librefm_finish_tx = scrobble_tx.clone();
    let scrobble_librefm_finish_ui = window.as_weak();
    window.on_scrobble_librefm_finish_requested(move || {
        if let Some(window) = scrobble_librefm_finish_ui.upgrade() {
            send_scrobble_command(
                &scrobble_librefm_finish_tx,
                &window,
                ScrobbleCommand::CompleteAuthorization(ScrobbleProvider::LibreFm),
                ScrobbleProvider::LibreFm,
            );
        }
    });

    let scrobble_librefm_disconnect_tx = scrobble_tx.clone();
    let scrobble_librefm_disconnect_ui = window.as_weak();
    window.on_scrobble_librefm_disconnect_requested(move || {
        if let Some(window) = scrobble_librefm_disconnect_ui.upgrade() {
            send_scrobble_command(
                &scrobble_librefm_disconnect_tx,
                &window,
                ScrobbleCommand::Disconnect(ScrobbleProvider::LibreFm),
                ScrobbleProvider::LibreFm,
            );
        }
    });

    let detect_now_tx = detector_tx.clone();
    let detect_now_ui = window.as_weak();
    window.on_detect_now_requested(move || {
        let Some(window) = detect_now_ui.upgrade() else {
            return;
        };
        let _ = detect_now_tx.send(media_detector::Control::Configure(detector_settings_from_ui(
            &window,
        )));
        if let Err(error) = detect_now_tx.send(media_detector::Control::DetectNow) {
            window.set_detection_status(format!("Could not start detection: {error}").into());
        }
    });

    let auto_detect_tx = detector_tx.clone();
    let auto_detect_ui = window.as_weak();
    window.on_auto_detect_changed(move |enabled| {
        let Some(window) = auto_detect_ui.upgrade() else {
            return;
        };
        let _ = auto_detect_tx.send(media_detector::Control::Configure(detector_settings_from_ui(
            &window,
        )));
        let control = if enabled {
            media_detector::Control::Start
        } else {
            media_detector::Control::Stop
        };
        if let Err(error) = auto_detect_tx.send(control) {
            window.set_detection_status(format!("Could not change auto-detect: {error}").into());
        }
    });

    let detector_settings_tx = detector_tx.clone();
    let detector_settings_ui = window.as_weak();
    window.on_detector_settings_changed(move || {
        if let Some(window) = detector_settings_ui.upgrade() {
            let _ = detector_settings_tx
                .send(media_detector::Control::Configure(detector_settings_from_ui(&window)));
        }
    });

    let open_url_ui = window.as_weak();
    window.on_open_url_requested(move |url| {
        if url.trim().is_empty() {
            return;
        }
        if let Err(error) = webbrowser::open(url.as_str())
            && let Some(window) = open_url_ui.upgrade()
        {
            window.set_detection_status(format!("Could not open link: {error}").into());
        }
    });

    let run_result = window.run();
    drop(window);
    oauth_stopping.store(true, Ordering::Relaxed);
    drop(rpc_tx);
    drop(gateway_tx);
    drop(oauth_tx);
    drop(scrobble_tx);
    drop(detector_tx);

    if rpc_worker.join().is_err() {
        eprintln!("Discord Rich Presence worker stopped unexpectedly");
    }
    if gateway_worker.join().is_err() {
        eprintln!("Discord bot Gateway worker stopped unexpectedly");
    }
    if oauth_worker.join().is_err() {
        eprintln!("Discord OAuth2 worker stopped unexpectedly");
    }
    if scrobble_worker.join().is_err() {
        eprintln!("Scrobbling worker stopped unexpectedly");
    }
    if detector_worker.join().is_err() {
        eprintln!("Media detector worker stopped unexpectedly");
    }

    run_result?;
    Ok(())
}

fn detector_settings_from_ui(window: &MainWindow) -> media_detector::Settings {
    media_detector::Settings {
        native: window.get_detect_native(),
        browser: window.get_detect_browser(),
        players: window.get_detect_players(),
        auto_update: window.get_auto_detect(),
        activity_type: window.get_media_activity_type().to_string(),
        show_album: window.get_media_show_album(),
        hide_paused: window.get_media_hide_paused(),
    }
}

fn restore_saved_token(window: &MainWindow) -> Result<(), String> {
    match token_store::load()? {
        Some(token) => {
            window.set_gateway_token(token.as_str().into());
            window.set_remember_gateway_token(true);
            Ok(())
        }
        None => Ok(()),
    }
}

fn set_scrobble_track(tx: &mpsc::Sender<ScrobbleCommand>, window: &MainWindow) {
    let track = match scrobble_track_from_ui(window) {
        Ok(track) => track,
        Err(error) => {
            window.set_scrobble_lastfm_status(error.clone().into());
            window.set_scrobble_listenbrainz_status(error.clone().into());
            window.set_scrobble_librefm_status(error.into());
            return;
        }
    };
    if let Err(error) = tx.send(ScrobbleCommand::SetTrack(track)) {
        let message = format!("Could not update scrobbling: {error}");
        window.set_scrobble_lastfm_status(message.clone().into());
        window.set_scrobble_listenbrainz_status(message.clone().into());
        window.set_scrobble_librefm_status(message.into());
    }
}

fn send_scrobble_command(
    tx: &mpsc::Sender<ScrobbleCommand>,
    window: &MainWindow,
    command: ScrobbleCommand,
    provider: ScrobbleProvider,
) {
    if let Err(error) = tx.send(command) {
        let message = format!("Could not start scrobbling action: {error}");
        match provider {
            ScrobbleProvider::LastFm => {
                window.set_scrobble_lastfm_busy(false);
                window.set_scrobble_lastfm_status(message.into());
            }
            ScrobbleProvider::ListenBrainz => {
                window.set_scrobble_listenbrainz_busy(false);
                window.set_scrobble_listenbrainz_status(message.into());
            }
            ScrobbleProvider::LibreFm => {
                window.set_scrobble_librefm_busy(false);
                window.set_scrobble_librefm_status(message.into());
            }
        }
    }
}

fn scrobble_track_from_ui(window: &MainWindow) -> Result<Option<ScrobbleTrack>, String> {
    if window.get_rpc_mode() != "Media RPC" {
        return Ok(None);
    }
    let kind = window.get_media_kind();
    if kind != "Song" && kind != "Music video" {
        return Ok(None);
    }
    let duration = parse_seconds(&window.get_media_duration(), "duration")?;
    let position = parse_seconds(&window.get_media_position(), "position")?;
    Ok(Some(ScrobbleTrack {
        title: window.get_media_title().to_string(),
        artist: window.get_media_artist().to_string(),
        album: window.get_media_album().to_string(),
        duration_seconds: (duration > 0).then_some(duration),
        position_seconds: position,
        playing: window.get_media_playing(),
    }))
}

fn presence_from_ui(window: &MainWindow) -> Result<PresenceDraft, String> {
    let is_media = window.get_rpc_mode() == "Media RPC";
    let (name, details, state, activity_type, playing, duration, position) = if is_media {
        let title = window.get_media_title().to_string();
        if title.trim().is_empty() {
            return Err("Enter a media title.".into());
        }
        let artist = window.get_media_artist().to_string();
        let album = window.get_media_album().to_string();
        let mut state = match (artist.trim().is_empty(), album.trim().is_empty()) {
            (false, false) => format!("{artist} · {album}"),
            (false, true) => artist,
            (true, false) => album,
            (true, true) => String::new(),
        };
        if !window.get_media_playing() {
            state = if state.is_empty() {
                "Paused".into()
            } else {
                format!("{state} · Paused")
            };
        }
        (
            "YouTube Music".to_owned(),
            title,
            state,
            2,
            window.get_media_playing(),
            parse_seconds(&window.get_media_duration(), "duration")?,
            parse_seconds(&window.get_media_position(), "position")?,
        )
    } else {
        (
            window.get_activity_name().to_string(),
            window.get_activity_details().to_string(),
            window.get_activity_state().to_string(),
            match window.get_activity_type().as_str() {
                "Listening" => 2,
                "Watching" => 3,
                "Competing" => 5,
                _ => 0,
            },
            false,
            0,
            0,
        )
    };

    let draft = PresenceDraft {
        name,
        details,
        state,
        activity_type,
        playing,
        position_seconds: position,
        duration_seconds: duration,
        large_image: window.get_large_image().to_string(),
        large_text: window.get_large_text().to_string(),
        button1_label: window.get_button1_label().to_string(),
        button1_url: window.get_button1_url().to_string(),
        button2_label: window.get_button2_label().to_string(),
        button2_url: window.get_button2_url().to_string(),
    };
    draft.validate()?;
    Ok(draft)
}

fn parse_seconds(value: &str, field: &str) -> Result<u64, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(0);
    }
    value
        .parse::<u64>()
        .map_err(|_| format!("Media {field} must be a non-negative whole number of seconds."))
}
