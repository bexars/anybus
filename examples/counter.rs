use std::{fmt::Display, time::Duration};

use msgbus::helper::ShutdownWithCtrlC;
use msgbus::{bus_uuid, Handle, MsgBus};
use tokio;
// use msgbus_macro::bus_uuid;
use uuid::Uuid;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[bus_uuid("018dce05-972c-7c2d-a5a1-579b828f7610")]

enum ChatMessage {
    Msg { from: Box<str>, text: Box<str> },
    Goodbye,
    Hello(Box<str>), // Name to display when joining
}

// impl msgbus::BusRider for ChatMessage {
//     fn default_uuid(&self) -> Uuid {
//         ChatMessage::get_uuid()
//     }
//     fn as_any(self: Box<Self>) -> Box<dyn std::any::Any> {
//         self
//     }
// }
// impl ChatMessage {
//     const fn get_uuid() -> Uuid {
//         const UUID: Uuid = Uuid::from_u128(2065520472143507018524053171180893712);
//         UUID
//     }
// }

impl Display for ChatMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatMessage::Msg { text, from } => {
                write!(f, "{:8} => {}", from, text)
            }
            ChatMessage::Goodbye => write!(f, "Goodbye"),
            ChatMessage::Hello(user) => write!(f, "{} has joined", *user),
        }
    }
}

// impl BusRider for ChatMessage {

//     fn default_uuid(&self) -> Uuid {
//         ChatMessage::get_uuid()
//     }

//     fn as_any(self: Box<Self>) -> Box<dyn std::any::Any> {
//         self
//     }
// }

// impl ChatMessage {
//     const fn get_uuid() -> Uuid {

//         const UUID: Uuid = Uuid::nil();
//         UUID
//     }
// }

struct ChatListener {}

impl ChatListener {
    async fn run(mut handle: Handle) {
        let mut listener = handle
            .register_anycast::<ChatMessage>(ChatMessage::get_uuid())
            .unwrap();
        println!("Entering chat listen loop");
        loop {
            let message = listener.recv().await;
            match message {
                Ok(msg) => println!("{msg}"),
                Err(e) => {
                    match e {
                        msgbus::errors::ReceiveError::ConnectionClosed => {
                            eprintln!("Chatlistener connection closed")
                        }
                        msgbus::errors::ReceiveError::RegistrationFailed => {
                            eprintln!("Registration failed")
                        }
                        msgbus::errors::ReceiveError::Shutdown => {
                            eprintln!("Chatlistener received shutdown")
                        }
                    }
                    eprintln!("{e}");
                    break;
                }
            }
        }
    }
}

async fn countdown(handle: Handle, name: Box<str>, mut count: isize) {
    handle.send(ChatMessage::Hello(name.clone())).unwrap();

    loop {
        let payload = if count > 0 {
            format!("{}", count)
        } else {
            "Boom!".into()
        };
        if let Result::Err(e) = handle.send(ChatMessage::Msg {
            from: name.clone(),
            text: payload.into_boxed_str(),
        }) {
            // match e {
            //     msgbus::errors::MsgBusHandleError::SendError(bus_rider) => todo!(),
            //     msgbus::errors::MsgBusHandleError::NoRoute => todo!(),
            //     msgbus::errors::MsgBusHandleError::SubscriptionFailed => todo!(),
            //     msgbus::errors::MsgBusHandleError::Shutdown => todo!(),
            // }
            dbg!(e);
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        count -= 1;
        if count < 0 {
            break;
        }
    }
}

#[cfg(target_family = "unix")]
#[tokio::main]
async fn main() {
    let (bus, handle) = MsgBus::new();

    #[cfg(target_family = "unix")]
    let bus = ShutdownWithCtrlC::from(bus);

    let cl = tokio::spawn(ChatListener::run(handle.clone()));
    std::thread::sleep(Duration::from_secs(1));

    let c1 = tokio::spawn(countdown(handle.clone(), "Alice".into(), 10));
    let c2 = tokio::spawn(countdown(handle.clone(), "Bob".into(), 30));
    let c3 = tokio::spawn(tokio::time::sleep(Duration::from_secs(15)));
    let _blah = tokio::join! { cl, c1, c2, c3 };
    bus.shutdown();
}
