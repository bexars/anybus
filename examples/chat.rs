use std::{fmt::Display, time::Duration};

use msgbus::helper::ShutdownWithCtrlC;
use msgbus::BusRider;
use tokio;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    let (control, mut handle) = msgbus::MsgBus::new();
    let _control = ShutdownWithCtrlC::from(control);
    let mut chat_listener = handle
        .register_anycast::<ChatMessage>(ChatMessage::get_uuid())
        .expect("Unable to register");

    tokio::time::sleep(Duration::from_millis(2000)).await;
    dbg!("Left first chat sleep");
    handle.send(ChatMessage::Hello("Matt".into())).unwrap();

    tokio::time::sleep(Duration::from_millis(2000)).await;

    // handle.send(ChatMessage::Goodbye).unwrap();

    loop {
        let msg = chat_listener.recv().await;

        dbg!(&msg);
        match msg {
            Ok(msg) => dbg!(msg),
            Err(e) => {

                dbg!(e);
                break;
            }
        };
    }

    // control.shutdown();
    match handle.send(ChatMessage::Goodbye) {
        Ok(_) => {}
        Err(e) => println!("{e}"),
    };
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
enum ChatMessage {
    Msg { from: String, text: String },
    Goodbye,
    Hello(String), // Name to display when joining
}

impl Display for ChatMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatMessage::Msg { text: _, from: _ } => {
                todo!()
            }
            ChatMessage::Goodbye => write!(f, "Goodbye"),
            ChatMessage::Hello(_) => todo!(),
        }
    }
}

impl BusRider for ChatMessage {
    fn default_uuid(&self) -> Uuid {
        ChatMessage::get_uuid()
    }

    fn as_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}

impl ChatMessage {
    const fn get_uuid() -> Uuid {
        const UUID: Uuid = Uuid::nil();
        UUID
    }
}
