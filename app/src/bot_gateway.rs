use std::{sync::mpsc::Receiver, thread, time::Duration};

use tokio::{runtime::Builder, sync::mpsc as async_mpsc};
use twilight_gateway::{CloseFrame, Event, EventTypeFlags, Intents, Shard, ShardId, StreamExt};
use twilight_model::gateway::{payload::outgoing::UpdatePresence, presence::Status};
use zeroize::Zeroizing;

use crate::{MainWindow, presence::PresenceDraft, token_store};

pub enum Command {
    Login {
        token: String,
        remember: bool,
        presence: PresenceDraft,
    },
    Logout,
    UpdatePresence(PresenceDraft),
}

struct PendingLogin {
    token: Zeroizing<String>,
    remember: bool,
}

enum Input {
    Command(Option<Command>),
    Event(Option<Result<Event, twilight_gateway::error::ReceiveMessageError>>),
}

pub fn run_worker(commands: Receiver<Command>, ui: slint::Weak<MainWindow>) {
    let runtime = match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            set_ui(
                &ui,
                false,
                false,
                format!("Could not start Gateway runtime: {error}"),
            );
            return;
        }
    };

    let (sender, receiver) = async_mpsc::unbounded_channel();
    let bridge = thread::Builder::new()
        .name("discord-gateway-commands".into())
        .spawn(move || {
            while let Ok(command) = commands.recv() {
                if sender.send(command).is_err() {
                    break;
                }
            }
        });

    if let Err(error) = bridge {
        set_ui(
            &ui,
            false,
            false,
            format!("Could not start Gateway command bridge: {error}"),
        );
        return;
    }

    runtime.block_on(run_gateway(receiver, ui));
}

async fn run_gateway(
    mut commands: async_mpsc::UnboundedReceiver<Command>,
    ui: slint::Weak<MainWindow>,
) {
    let mut shard: Option<Shard> = None;
    let mut pending_login: Option<PendingLogin> = None;
    let mut current_presence: Option<PresenceDraft> = None;
    let mut connected = false;

    loop {
        let input = if let Some(active_shard) = shard.as_mut() {
            tokio::select! {
                command = commands.recv() => Input::Command(command),
                event = active_shard.next_event(EventTypeFlags::all()) => Input::Event(event),
            }
        } else {
            Input::Command(commands.recv().await)
        };

        match input {
            Input::Command(Some(Command::Login {
                token,
                remember,
                presence,
            })) => {
                if let Err(message) = presence.validate() {
                    set_ui(&ui, false, false, message);
                    continue;
                }
                if token.trim().is_empty() {
                    set_ui(&ui, false, false, "Enter a Discord bot token.".into());
                    continue;
                }

                if let Some(active_shard) = shard.as_mut() {
                    stop_shard(active_shard).await;
                }

                current_presence = Some(presence);
                pending_login = Some(PendingLogin {
                    token: Zeroizing::new(token.trim().to_owned()),
                    remember,
                });
                let pending = pending_login.as_ref().expect("pending login was set");
                shard = Some(Shard::new(
                    ShardId::ONE,
                    pending.token.to_string(),
                    Intents::empty(),
                ));
                connected = false;
                set_ui(
                    &ui,
                    false,
                    true,
                    "Connecting to Discord Gateway as a bot...".into(),
                );
            }
            Input::Command(Some(Command::Logout)) => {
                if let Some(active_shard) = shard.as_mut() {
                    stop_shard(active_shard).await;
                }
                shard = None;
                pending_login = None;
                current_presence = None;
                connected = false;
                let status = match token_store::clear() {
                    Ok(()) => "Logged out; saved bot credentials were removed.".into(),
                    Err(error) => {
                        format!("Logged out, but saved credentials could not be removed: {error}")
                    }
                };
                set_ui(&ui, false, false, status);
            }
            Input::Command(Some(Command::UpdatePresence(presence))) => {
                if let Err(message) = presence.validate() {
                    set_ui(&ui, connected, false, message);
                    continue;
                }
                current_presence = Some(presence);
                if connected
                    && let (Some(active_shard), Some(current)) =
                        (shard.as_ref(), current_presence.as_ref())
                {
                    send_presence(active_shard, current);
                    set_ui(&ui, true, false, "Bot activity updated.".into());
                }
            }
            Input::Command(None) => break,
            Input::Event(Some(Ok(Event::Ready(_)))) => {
                connected = true;
                if let Some(pending) = pending_login.take() {
                    let persistence_status = if pending.remember {
                        token_store::save(&pending.token)
                    } else {
                        token_store::clear()
                    };
                    if let Err(error) = persistence_status {
                        set_ui(
                            &ui,
                            true,
                            false,
                            format!("Bot logged in, but token storage failed: {error}"),
                        );
                    } else {
                        set_ui(
                            &ui,
                            true,
                            false,
                            "Logged in as bot. The token is stored encrypted in the OS credential store.".into(),
                        );
                    }
                } else {
                    set_ui(&ui, true, false, "Bot Gateway is ready.".into());
                }
                if let (Some(active_shard), Some(presence)) =
                    (shard.as_ref(), current_presence.as_ref())
                {
                    send_presence(active_shard, presence);
                }
            }
            Input::Event(Some(Ok(Event::GatewayClose(_)))) => {
                set_ui(
                    &ui,
                    connected,
                    !connected,
                    "Discord Gateway closed the connection; the client will attempt to reconnect."
                        .into(),
                );
            }
            Input::Event(Some(Ok(_))) => {}
            Input::Event(Some(Err(_))) => {
                set_ui(
                    &ui,
                    connected,
                    !connected,
                    "A Discord Gateway event could not be read; reconnecting if possible.".into(),
                );
            }
            Input::Event(None) => {
                shard = None;
                pending_login = None;
                connected = false;
                set_ui(
                    &ui,
                    false,
                    false,
                    "Discord Gateway connection ended.".into(),
                );
            }
        }
    }

    if let Some(active_shard) = shard.as_mut() {
        stop_shard(active_shard).await;
    }
}

fn send_presence(shard: &Shard, draft: &PresenceDraft) {
    match UpdatePresence::new(vec![draft.gateway_activity()], false, None, Status::Online) {
        Ok(presence) => shard.command(&presence),
        Err(error) => eprintln!("Could not build Discord bot presence: {error}"),
    }
}

async fn stop_shard(shard: &mut Shard) {
    shard.close(CloseFrame::NORMAL);
    let _ = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = shard.next_event(EventTypeFlags::all()).await {
            if matches!(event, Ok(Event::GatewayClose(_))) {
                break;
            }
        }
    })
    .await;
}

fn set_ui(ui: &slint::Weak<MainWindow>, connected: bool, connecting: bool, status: String) {
    let ui = ui.clone();
    if let Err(error) = slint::invoke_from_event_loop(move || {
        if let Some(window) = ui.upgrade() {
            window.set_gateway_connected(connected);
            window.set_gateway_connecting(connecting);
            window.set_gateway_status(status.into());
            if connected {
                window.set_gateway_token("".into());
            }
        }
    }) {
        eprintln!("Could not update bot Gateway status: {error}");
    }
}
