mod bot_gateway;
mod discord_rpc;
mod presence;
mod token_store;

use bot_gateway::Command as GatewayCommand;
use discord_rpc::Command as RpcCommand;
use presence::{PresenceDraft, youtube_music_search_url};
use std::{sync::mpsc, thread};

slint::include_modules!();

fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    let connect_tx = rpc_tx.clone();
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

    let run_result = window.run();
    drop(window);
    drop(rpc_tx);
    drop(gateway_tx);

    if rpc_worker.join().is_err() {
        eprintln!("Discord Rich Presence worker stopped unexpectedly");
    }
    if gateway_worker.join().is_err() {
        eprintln!("Discord bot Gateway worker stopped unexpectedly");
    }

    run_result?;
    Ok(())
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
