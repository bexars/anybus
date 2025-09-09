mod chatview;
use std::collections::HashMap;

use anybus::bus_uuid;
use chatview::ChatViewWidget;
use color_eyre::Result;
use serde::{Deserialize, Serialize};
use tokio::select;
use tui_textarea::TextArea;
use uuid::Uuid;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    let file_appender = tracing_appender::rolling::hourly("./logs/", "tui-chat.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_max_level(tracing::Level::TRACE)
        .init();
    color_eyre::install()?;
    // let mut terminal = ratatui::init();
    let app_result = App::default().run().await;
    ratatui::restore();
    app_result
}

mod tui;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[bus_uuid("123e4567-e89b-12d3-a456-426614174010")]
pub enum ChatMessage {
    Chat(User, String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    nickname: String,
    id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DirectMessage {
    from: User,
    message: String,
}

pub enum Action {
    Quit,
    ProcessInput,
    ScrollUp,
    ScrollDown,
    AddMessage(String),
}

#[derive(Debug)]
struct App {
    should_quit: bool,
    input: TextArea<'static>,
    history: ChatViewWidget,
    scroll_state: tui_scrollview::ScrollViewState,
    id: Uuid,
    nickname: String,
    bus: anybus::AnyBus,
    chat_members: HashMap<String, User>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            should_quit: false,
            input: TextArea::default(),
            history: ChatViewWidget::default(),
            scroll_state: tui_scrollview::ScrollViewState::default(),
            id: Uuid::now_v7(),
            nickname: "Anonymous".to_string(),
            bus: anybus::AnyBus::new(),
            chat_members: HashMap::new(),
        }
    }
}

impl App {
    async fn run(&mut self) -> Result<()> {
        let mut tui = tui::Tui::new()?
            .tick_rate(4.0) // 4 ticks per second
            .frame_rate(30.0); // 30 frames per second

        tui.enter()?; // Starts event handler, enters raw mode, enters alternate screen
        let mut chat_listener = self
            .bus
            .handle()
            .clone()
            .register_broadcast()
            .await
            .unwrap();

        let mut dm_listener = self
            .bus
            .handle()
            .clone()
            .register_anycast_uuid::<DirectMessage>(self.id)
            .await
            .unwrap();

        loop {
            tui.draw(|f| {
                // Deref allows calling `tui.terminal.draw`
                self.ui(f);
            })?;

            select! {
                Ok(msg) = chat_listener.recv() => {
                    let mut maybe_action = self.process_anybusmsg(msg);
                    while let Some(action) = maybe_action {
                        maybe_action = self.update(action);
                    }
                }

                Ok(msg) = dm_listener.recv() => {
                    let dm = format!("(DM) {}: {}", msg.from.nickname, msg.message);
                    let mut maybe_action = Some(Action::AddMessage(dm));
                    while let Some(action) = maybe_action {
                        maybe_action = self.update(action);
                    }
                }
                Some(evt) = tui.next() => {
                    // `tui.next().await` blocks till next event
                    let mut maybe_action = self.handle_event(evt);
                    while let Some(action) = maybe_action {
                        maybe_action = self.update(action);
                    }
                }
                else => break,

            };

            if self.should_quit {
                break;
            }
        }

        tui.exit()?; // stops event handler, exits raw mode, exits alternate screen

        Ok(())
    }

    fn update(&mut self, action: Action) -> Option<Action> {
        match action {
            Action::Quit => {
                self.should_quit = true;
                self.bus.shutdown();
                None
            }
            Action::ProcessInput => {
                let lines = self.input.lines();
                let message = lines.join("\n");
                let message = message.trim();
                if message.is_empty() {
                    return None;
                }

                if message.starts_with('/') {
                    let mut parts = message.splitn(2, ' ');
                    let command = parts.next().unwrap_or("");
                    let argument = parts.next().unwrap_or("");
                    match command {
                        "/nick" => {
                            if !argument.is_empty() {
                                self.nickname = argument.to_string();
                                self.history
                                    .content
                                    .push(format!("* You are now known as {}", self.nickname));
                            } else {
                                self.history
                                    .content
                                    .push("* Usage: /nick <new_nickname>".to_string());
                            }
                        }
                        "/dm" => {
                            let mut arg_parts = argument.splitn(2, ' ');
                            let target_nick = arg_parts.next().unwrap_or("");
                            let dm_message = arg_parts.next().unwrap_or("");
                            if target_nick.is_empty() || dm_message.is_empty() {
                                self.history
                                    .content
                                    .push("* Usage: /dm <nickname> <message>".to_string());
                            } else if let Some(target_user) = self.chat_members.get(target_nick) {
                                let dm = DirectMessage {
                                    from: User {
                                        nickname: self.nickname.clone(),
                                        id: self.id,
                                    },
                                    message: dm_message.to_string(),
                                };
                                if self.bus.handle().send_to_uuid(target_user.id, dm).is_err() {
                                    return Action::Quit.into();
                                };
                                self.history.content.push(format!(
                                    "* (DM to {}): {}",
                                    target_user.nickname, dm_message
                                ));
                            } else {
                                self.history
                                    .content
                                    .push(format!("* No such user: {}", target_nick));
                            }
                        }
                        _ => {
                            self.history
                                .content
                                .push(format!("* Unknown command: {}", command));
                        }
                    }
                    self.input = TextArea::default();
                    return None;
                }

                if self
                    .bus
                    .handle()
                    .send(ChatMessage::Chat(
                        User {
                            nickname: self.nickname.clone(),
                            id: self.id,
                        },
                        message.into(),
                    ))
                    .is_err()
                {
                    return Action::Quit.into();
                };
                self.input = TextArea::default();
                None
            }
            Action::ScrollUp => {
                self.scroll_state.scroll_up();
                None
            }
            Action::ScrollDown => {
                self.scroll_state.scroll_down();
                None
            }
            Action::AddMessage(chat_msg) => {
                self.history.content.push(chat_msg);
                self.scroll_state.scroll_to_bottom();
                None
            }
        }
    }

    fn process_anybusmsg(&mut self, msg: ChatMessage) -> Option<Action> {
        match msg {
            ChatMessage::Chat(user, message) => {
                self.chat_members
                    .insert(user.nickname.clone(), user.clone());
                let chat_msg = format!("{}: {}", user.nickname, message);
                Some(Action::AddMessage(chat_msg))
            }
        }
    }

    fn handle_event(&mut self, evt: tui::Event) -> Option<Action> {
        match evt {
            tui::Event::Key(key_event)
                if key_event.kind == crossterm::event::KeyEventKind::Press =>
            {
                match key_event.code {
                    crossterm::event::KeyCode::Esc => Some(Action::Quit),
                    crossterm::event::KeyCode::Enter => Some(Action::ProcessInput),
                    crossterm::event::KeyCode::Up => Some(Action::ScrollUp),
                    crossterm::event::KeyCode::Down => Some(Action::ScrollDown),

                    _ => {
                        self.input.input(key_event);
                        None
                    } // _ => None,
                }
            }
            tui::Event::Tick => None,
            _ => None,
        }
    }

    fn ui(&mut self, f: &mut ratatui::Frame) {
        let size = f.area();
        let layout = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .margin(1)
            .constraints(
                [
                    ratatui::layout::Constraint::Fill(1),
                    ratatui::layout::Constraint::Length(3),
                ]
                .as_ref(),
            )
            .split(size);

        let chat_log = ratatui::widgets::Block::default()
            .title("Chat History")
            .borders(ratatui::widgets::Borders::ALL);

        let input_block = ratatui::widgets::Block::default()
            .title("Input")
            .borders(ratatui::widgets::Borders::ALL);
        let text_area = input_block.inner(layout[1]);
        // let chat_view = ChatViewWidget {
        //     content: self.history.lines().map(|s| s.to_string()).collect(),
        // };
        let history_widget = ChatViewWidget {
            content: self.history.content.clone(),
        };

        let history_area = chat_log.inner(layout[0]);

        f.render_widget(chat_log, layout[0]);
        f.render_stateful_widget(history_widget, history_area, &mut self.scroll_state);

        f.render_widget(input_block, layout[1]);
        f.render_widget(&self.input, text_area);
    }
}
