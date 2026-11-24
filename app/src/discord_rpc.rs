use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use std::sync::mpsc;

use crate::{MainWindow, presence::PresenceDraft};

pub enum Command {
    Connect {
        application_id: String,
        presence: PresenceDraft,
    },
    Update(PresenceDraft),
    /// Clear the activity when no RPC mode is enabled.
    Clear,
    Disconnect,
}

pub fn run_worker(commands: mpsc::Receiver<Command>, ui: slint::Weak<MainWindow>) {
    let mut client: Option<DiscordIpcClient> = None;

    while let Ok(command) = commands.recv() {
        match command {
            Command::Connect {
                application_id,
                presence,
            } => {
                set_ui_state(&ui, false, true, "Connecting to Discord desktop...".into());

                if let Err(message) = validate(&application_id, &presence) {
                    set_ui_state(&ui, false, false, message);
                    continue;
                }

                if let Some(mut connected) = client.take()
                    && let Err(error) = close_client(&mut connected)
                {
                    eprintln!("Failed to close previous Discord IPC connection: {error}");
                }

                let mut next = DiscordIpcClient::new(application_id.trim());
                if let Err(error) = next.connect() {
                    set_ui_state(
                        &ui,
                        false,
                        false,
                        format!(
                            "Could not connect to Discord desktop. Make sure Discord is running: {error}"
                        ),
                    );
                    continue;
                }

                if let Err(error) = next.set_activity(presence.ipc_activity()) {
                    let close_error = next.close().err();
                    let message = match close_error {
                        Some(close_error) => {
                            format!(
                                "Could not set activity: {error}; closing IPC also failed: {close_error}"
                            )
                        }
                        None => format!("Could not set activity: {error}"),
                    };
                    set_ui_state(&ui, false, false, message);
                    continue;
                }

                client = Some(next);
                set_ui_state(&ui, true, false, "Connected to Discord desktop".into());
            }
            Command::Update(presence) => {
                if let Err(message) = validate_presence(&presence) {
                    set_ui_state(&ui, client.is_some(), false, message);
                    continue;
                }
                let Some(connected) = client.as_mut() else {
                    continue;
                };
                match connected.set_activity(presence.ipc_activity()) {
                    Ok(()) => set_ui_state(&ui, true, false, "Desktop activity updated.".into()),
                    Err(error) => {
                        set_ui_state(
                            &ui,
                            false,
                            false,
                            format!("Could not update desktop activity: {error}"),
                        );
                        if let Some(mut disconnected) = client.take()
                            && let Err(close_error) = close_client(&mut disconnected)
                        {
                            eprintln!("Failed to close disconnected Discord IPC: {close_error}");
                        }
                    }
                }
            }
            Command::Clear => {
                if let Some(connected) = client.as_mut()
                    && let Err(error) = connected.clear_activity()
                {
                    set_ui_state(
                        &ui,
                        false,
                        false,
                        format!("Could not clear desktop activity: {error}"),
                    );
                }
            }
            Command::Disconnect => {
                set_ui_state(&ui, false, true, "Disconnecting...".into());
                let result = match client.take() {
                    Some(mut connected) => close_client(&mut connected),
                    None => Ok(()),
                };
                match result {
                    Ok(()) => set_ui_state(&ui, false, false, "Disconnected".into()),
                    Err(error) => {
                        set_ui_state(
                            &ui,
                            false,
                            false,
                            format!("Disconnected with an error: {error}"),
                        );
                    }
                }
            }
        }
    }

    if let Some(mut connected) = client
        && let Err(error) = close_client(&mut connected)
    {
        eprintln!("Failed to close Discord IPC during shutdown: {error}");
    }
}

fn close_client(client: &mut DiscordIpcClient) -> Result<(), String> {
    let clear_result = client.clear_activity().map_err(|error| error.to_string());
    let close_result = client.close().map_err(|error| error.to_string());

    match (clear_result, close_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(clear_error), Ok(())) => Err(format!("could not clear activity: {clear_error}")),
        (Ok(()), Err(close_error)) => Err(format!("could not close IPC: {close_error}")),
        (Err(clear_error), Err(close_error)) => Err(format!(
            "could not clear activity: {clear_error}; could not close IPC: {close_error}"
        )),
    }
}

fn set_ui_state(ui: &slint::Weak<MainWindow>, connected: bool, connecting: bool, message: String) {
    let ui = ui.clone();
    if let Err(error) = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            window.set_connected(connected);
            window.set_connecting(connecting);
            window.set_connection_status(message.into());
        }
    }) {
        eprintln!("Could not update the Discord connection status: {error}");
    }
}

fn validate(application_id: &str, presence: &PresenceDraft) -> Result<(), String> {
    let application_id = application_id.trim();
    if application_id.is_empty() || !application_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Enter the numeric Application ID from the Discord Developer Portal.".into());
    }
    validate_presence(presence)
}

fn validate_presence(presence: &PresenceDraft) -> Result<(), String> {
    presence.validate()?;
    if presence.details.trim().is_empty() && presence.state.trim().is_empty() {
        return Err("Enter activity details or state before connecting.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate, validate_presence};
    use crate::presence::PresenceDraft;

    fn presence() -> PresenceDraft {
        PresenceDraft {
            name: "Music Presence".into(),
            details: "Listening to music".into(),
            state: String::new(),
            activity_type: 2,
            playing: false,
            position_seconds: 0,
            duration_seconds: 0,
            large_image: String::new(),
            large_text: String::new(),
            details_url: String::new(),
            state_url: String::new(),
            large_url: String::new(),
            small_url: String::new(),
            status_display_type: String::new(),
            button1_label: String::new(),
            button1_url: String::new(),
            button2_label: String::new(),
            button2_url: String::new(),
        }
    }

    #[test]
    fn requires_a_numeric_application_id() {
        assert!(validate("123456789012345678", &presence()).is_ok());
        assert!(validate("not-an-id", &presence()).is_err());
        assert!(validate("", &presence()).is_err());
    }

    #[test]
    fn rejects_empty_or_invalid_activity() {
        let mut invalid = presence();
        invalid.details.clear();
        assert!(validate_presence(&invalid).is_err());

        invalid.details = "Listening".into();
        invalid.button1_label = "Open".into();
        assert!(validate_presence(&invalid).is_err());
    }
}
