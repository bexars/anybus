use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::LazyLock;
use std::time::Duration;

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
            dm_msg = dm_rx.recv() => {
                match dm_msg {
                    Ok(dm_msg) => {
                        CHAT_MEMBERS.add_member(dm_msg.from.clone());
                        println!("(DM from {}): {}", dm_msg.from.nickname, dm_msg.message)
                    }
                    Err(e) => {
                        eprintln!("DM Listener error: {:?}", e);
                        break;
                    }
                }
            },
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
    let mut name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Anonymous".into());

    let mut bus = AnyBus::build()
        .enable_ctrlc_shutdown(true)
        .enable_ipc(true)
        .run();

    let handle = bus.handle().clone();

    // Start the receiver in the background
    let _listener = tokio::spawn(listener(handle.clone()));

    // Give the bus a moment to start
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Announce ourselves
    let _ = handle.send(ChatMessage::Hello(NickName {
        nickname: name.clone(),
        anybus_id: *ANYBUS_ID,
    }));

    println!("Simple AnyBus chat (type and press Enter, /quit to exit)\n");

    let mut line = String::new();
    loop {
        // print!("{}> ", name);
        io::stdout().flush().unwrap();

        line.clear();
        if io::stdin().read_line(&mut line).is_err() {
            break;
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
