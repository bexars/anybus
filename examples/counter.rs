use std::{
    env::{self},
    fmt::Display,
    time::Duration,
};

// use msgbus::helper::ShutdownWithCtrlC;
use msgbus::{Handle, MsgBus, bus_uuid};
use tokio;

use tracing_subscriber;

// use msgbus_macro::bus_uuid;
// use uuid::Uuid;

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
#[bus_uuid("018dce05-972c-7c2d-a5a1-579b828f7610")]

enum ChatMessage {
    Msg { from: String, text: String },
    Goodbye,
    Hello(String), // Name to display when joining
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
        let mut listener = handle.register_anycast::<ChatMessage>().await.unwrap();
        println!("Entering chat listen loop");
        loop {
            let message = listener.recv().await;
            match message {
                Ok(msg) => println!("Chatlistener msg: {msg}"),
                Err(e) => {
                    match e {
                        msgbus::errors::ReceiveError::ConnectionClosed => {
                            println!("Chatlistener connection closed")
                        }
                        msgbus::errors::ReceiveError::RegistrationFailed(_) => {
                            println!("Registration failed")
                        }
                        msgbus::errors::ReceiveError::Shutdown => {
                            println!("Chatlistener received shutdown");
                            return;
                        }
                    }
                    eprintln!("{e}");
                    break;
                }
            }
        }
    }
}

async fn countdown(handle: Handle, name: String, mut count: isize) {
    _ = handle.send(ChatMessage::Hello(name.clone()));

    loop {
        let payload = if count > 0 {
            format!("{}", count)
        } else {
            "Boom!".into()
        };
        if let Result::Err(_e) = handle.send(ChatMessage::Msg {
            from: name.clone(),
            text: payload.into(),
        }) {}
        tokio::time::sleep(Duration::from_secs(1)).await;
        count -= 1;
        if count < 0 {
            break;
        }
    }
}

// #[cfg(target_family = "unix")]
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // tracing_subscriber::fmt()
    //     .event_format(
    //         tracing_subscriber::fmt::format()
    //             .with_file(true)
    //             .with_line_number(true),
    //     )
    //     .init();
    let name = env::args().skip(1).next().unwrap();

    // let (bus, handle) = MsgBus::new();
    let mut bus = MsgBus::new();
    let handle = bus.handle().clone();
    // #[cfg(target_family = "unix")]
    // let bus = ShutdownWithCtrlC::from(bus);

    let cl = if name == "Alice" {
        tokio::spawn(ChatListener::run(handle.clone()))
    } else {
        tokio::spawn(async {})
    };
    std::thread::sleep(Duration::from_secs(2));

    let c1 = tokio::spawn(countdown(handle.clone(), name, 10));
    // let c2 = tokio::spawn(countdown(handle.clone(), "Bob".into(), 15));
    let c3 = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(12)).await;
        bus.shutdown();
    });
    let _blah = tokio::join! { cl, c1, c3 };
    println!("After join()");
    // bus.shutdown();
}
