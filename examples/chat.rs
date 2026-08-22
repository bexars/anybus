//: A simple chat example using AnyBus. This example demonstrates how to use AnyBus for a simple chat application,
//: including broadcasting messages, direct messaging, and using the ctrl-c watcher.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::LazyLock;
use std::time::Duration;

use tokio::sync::mpsc;

use anybus::{AnyBus, Handle, bus_uuid};
use std::sync::Mutex;
use tokio;
use uuid::Uuid;

static CHAT_MEMBERS: LazyLock<ChatMembers> = LazyLock::new(|| ChatMembers::new());
static ANYBUS_ID: LazyLock<Uuid> = LazyLock::new(|| Uuid::new_v4());

struct ChatMembers {
    members: Mutex<HashMap<Uuid, String>>,
}

impl ChatMembers {
    fn new() -> Self {
        Self {
            members: Mutex::new(HashMap::new()),
        }
    }

    fn add_member(&self, nickname: NickName) {
        let mut members = self.members.lock().unwrap();
        members.insert(nickname.anybus_id, nickname.nickname);
    }

    fn remove_member(&self, nickname: &NickName) {
        let mut members = self.members.lock().unwrap();
        members.remove(&nickname.anybus_id);
    }

    fn find_by_name(&self, nickname: &str) -> Option<NickName> {
        let members = self.members.lock().unwrap();
        members.iter().find_map(|(id, name)| {
            if name == nickname {
                Some(NickName {
                    nickname: name.clone(),
                    anybus_id: *id,
                })
            } else {
                None
            }
        })
    }

    fn find_by_id(&self, anybus_id: Uuid) -> Option<NickName> {
        let members = self.members.lock().unwrap();
        members.get(&anybus_id).map(|nickname| NickName {
            nickname: nickname.clone(),
            anybus_id,
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[bus_uuid("018dce05-972c-7c2d-a5a1-579b828f7611")]
enum ChatMessage {
    Hello(NickName),
    ChangeNick(NickName),
    Msg { from: NickName, text: String },
    Goodbye(Uuid), // Include the AnyBus ID of the user leaving
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DMessage {
    from: NickName,
    to: NickName,
    message: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct NickName {
    nickname: String,
    anybus_id: Uuid,
}

impl std::fmt::Display for ChatMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatMessage::Hello(NickName { nickname, .. }) => write!(f, "*** {} joined", nickname),
            ChatMessage::Msg { from, text } => write!(f, "{:8} | {}", from.nickname, text),
            ChatMessage::Goodbye(uuid) => write!(
                f,
                "*** {} left",
                CHAT_MEMBERS
                    .find_by_id(*uuid)
                    .map_or("Unknown".to_string(), |n| n.nickname)
            ),
            ChatMessage::ChangeNick(nick_name) => {
                let old_nick = match CHAT_MEMBERS.find_by_id(nick_name.anybus_id) {
                    Some(n) => n.nickname,
                    None => "Unknown".to_string(),
                };
                write!(
                    f,
                    "*** {} changed nickname to {}",
                    old_nick, nick_name.nickname
                )
            }
        }
    }
}

async fn listener(handle: Handle) {
    // Register for the group chat, the UUID is defined on the ChatMessage struct
    let mut rx = handle.register_broadcast::<ChatMessage>().await.unwrap();

    // Register the random UUID we created in the static on startup, this will receive Direct Messages
    let mut dm_rx = handle
        .register_unicast_uuid::<DMessage>(ANYBUS_ID.clone())
        .await
        .unwrap();
    println!("Listening for chat messages...\n");

    loop {
        tokio::select! {
            // Listen for direct messages and handle them
            dm_msg = dm_rx.recv() => {
                match dm_msg {
                    Ok(dm_msg) => {
                        // Add the sender to the chat members list if not already present
                        CHAT_MEMBERS.add_member(dm_msg.from.clone());
                        println!("(DM from {}): {}", dm_msg.from.nickname, dm_msg.message)
                    }
                    Err(e) => {
                        eprintln!("DM Listener error: {:?}", e);
                        break;
                    }
                }
            },
            // Listen for broadcast messages and handle them
            msg = rx.recv() => {
                match msg {
                    Ok(msg) => {
                        println!("{}", &msg);
                        handle_msg(&msg)
                    }
                    Err(e) => {
                        eprintln!("Listener error: {:?}", e);
                        break;
                    }

                }
            }
        }
    }
}

// Handle incoming messages and update the chat members list accordingly
fn handle_msg(msg: &ChatMessage) {
    match msg {
        ChatMessage::Hello(nick_name) => {
            CHAT_MEMBERS.add_member(nick_name.clone());
        }
        ChatMessage::Goodbye(uuid) => {
            if let Some(nick_name) = CHAT_MEMBERS.find_by_id(*uuid) {
                CHAT_MEMBERS.remove_member(&nick_name);
            }
        }
        ChatMessage::ChangeNick(nick_name) => {
            CHAT_MEMBERS.add_member(nick_name.clone());
        }
        ChatMessage::Msg { from, text: _ } => {
            CHAT_MEMBERS.add_member(from.clone());
        }
    }
}

#[tokio::main]
async fn main() {
    // Get the nickname from the command line arguments, default to "Anonymous" if not provided
    let mut name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Anonymous".into());

    // Create a new AnyBus instance, enabling Ctrl-C shutdown and IPC
    let mut bus = AnyBus::build()
        .enable_ctrlc_shutdown(true)
        .enable_ipc(true)
        .run();

    // Get a handle to the bus so we can send messages
    let handle = bus.handle().clone();

    // Get a receiver for AnyBus status messages, which will notify us of bus shutdowns
    let mut bus_status = handle
        .get_anybus_status_receiver()
        .await
        .expect("Couldn't get status receiver");

    // Start the receiver in the background
    let _listener = tokio::spawn(listener(handle.clone()));

    // Give the bus a moment to start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Announce ourselves
    let _ = handle.send(ChatMessage::Hello(NickName {
        nickname: name.clone(),
        anybus_id: *ANYBUS_ID,
    }));

    // Create a channel to receive lines from stdin
    let (tx, mut rx) = mpsc::channel::<String>(32);

    // Dedicated blocking thread for stdin so we can not block the main loop
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut lines = stdin.lock().lines();
        while let Some(Ok(line)) = lines.next() {
            if tx.blocking_send(line).is_err() {
                break; // receiver dropped → stop
            }
        }
    });

    println!("Simple AnyBus chat (type and press Enter, /quit to exit)\n");

    let mut line = String::new();
    loop {
        // print!("{}> ", name);
        io::stdout().flush().unwrap();

        line.clear();

        tokio::select! {
            Some(input) = rx.recv() => {
                line = input;
            }
            Ok(status) = bus_status.recv() => {
                println!("Bus status changed: {:?}", status);
                if status == anybus::AnyBusStatusMsg::ShuttingDown {
                    let _ = handle.send(ChatMessage::Goodbye(*ANYBUS_ID));
                    break
                }
            }
            else =>  break
        }

        let text = line.trim();
        if text.is_empty() {
            continue;
        }

        if !text.starts_with('/') {
            let _ = handle.send(ChatMessage::Msg {
                from: NickName {
                    nickname: name.clone(),
                    anybus_id: *ANYBUS_ID,
                },
                text: text.to_string(),
            });
            continue;
        }

        match text.split_whitespace().collect::<Vec<&str>>().as_slice() {
            ["/nick", nick] => {
                name = nick.to_string();
                let new_nick = NickName {
                    nickname: nick.to_string(),
                    anybus_id: *ANYBUS_ID,
                };
                CHAT_MEMBERS.remove_member(&NickName {
                    nickname: name.clone(),
                    anybus_id: *ANYBUS_ID,
                });
                CHAT_MEMBERS.add_member(new_nick.clone());
                let _ = handle.send(ChatMessage::ChangeNick(new_nick));
                continue;
            }
            ["/dm", nick, msg @ ..] => {
                let Some(target) = CHAT_MEMBERS.find_by_name(&nick) else {
                    println!("User '{}' not found.", nick);
                    continue;
                };
                println!("Sending DM to {}: {}", nick, target.anybus_id);
                handle
                    .send_to_uuid(
                        target.anybus_id,
                        DMessage {
                            from: NickName {
                                nickname: name.clone(),
                                anybus_id: *ANYBUS_ID,
                            },
                            to: target.clone(),
                            message: msg.join(" "),
                        },
                    )
                    .expect("Error sending to Uuid");

                continue;
            }

            ["/list"] => {
                let members = CHAT_MEMBERS.members.lock().unwrap();
                println!("Current members:");
                for (_id, name) in members.iter() {
                    println!("- {}", name);
                }
                continue;
            }
            ["/quit"] => {
                let _ = handle.send(ChatMessage::Goodbye(*ANYBUS_ID));
                break;
            }
            _ => {
                println!("Commands:");
                println!("/help - Show this help message");
                println!("/nick <new_nickname> - Change your nickname");
                println!("/list - List current members");
                println!("/dm <nickname> <message> - Send a direct message to a user");
                println!("/quit - Exit the chat");
                continue;
            }
        }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    bus.shutdown();
}
