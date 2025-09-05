use msgbus::BusRiderRpc;
use msgbus::Handle;
use msgbus::bus_uuid;
use serde::{Deserialize, Serialize};
// use std::time::Duration;

// use tokio::time::timeout;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[bus_uuid("123e4567-e89b-12d3-a456-426614174000")]
struct NumberMessage {
    pub value: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[bus_uuid("123e4567-e89b-12d3-a456-426614174001")]
struct StringMessage {
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[bus_uuid("123e4567-e89b-12d3-a456-426614174002")]
struct RpcMessage {
    pub value: u8,
}

impl BusRiderRpc for RpcMessage {
    type Response = RpcResponse;
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct RpcResponse {
    pub value: i32,
}

#[allow(dead_code)]
fn setup() -> (Handle, Handle) {
    let mb1 = msgbus::MsgBus::new();

    let mb2 = msgbus::MsgBus::new();
    (mb1.handle().clone(), mb2.handle().clone())
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_unicast_local_message_sending() {
    let mb1 = msgbus::MsgBus::new();
    let mut handle1 = mb1.handle().clone();
    let mut listener = handle1.register_unicast().await.unwrap();

    handle1.send(NumberMessage { value: 100 }).unwrap();
    let msg: NumberMessage = listener.recv().await.unwrap();
    assert_eq!(msg.value, 100);
}

#[cfg(feature = "tokio")]
#[tokio::test]
#[should_panic]

async fn test_unicast_local_one_registration_allowed() {
    let mb1 = msgbus::MsgBus::new();
    let mut handle1 = mb1.handle().clone();
    let _listener = handle1.register_unicast::<NumberMessage>().await.unwrap();
    let mut handle2 = mb1.handle().clone();
    let mut listener2 = handle2.register_unicast::<NumberMessage>().await.unwrap();
    handle1.send(NumberMessage { value: 100 }).unwrap();
    let msg: NumberMessage = listener2.recv().await.unwrap();
    assert_eq!(msg.value, 100);
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_unicast_local_two_handles() {
    let mb1 = msgbus::MsgBus::new();
    let mut handle1 = mb1.handle().clone();
    let _listener = handle1.register_unicast::<NumberMessage>().await.unwrap();
    let _handle2 = mb1.handle().clone();
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_unicast_local_two_message_types_one_handle() {
    let mb1 = msgbus::MsgBus::new();
    let mut handle1 = mb1.handle().clone();
    let handle2 = mb1.handle().clone();

    let mut listener1 = handle1.register_unicast::<NumberMessage>().await.unwrap();
    let mut listener2 = handle1.register_unicast::<StringMessage>().await.unwrap();
    handle2.send(NumberMessage { value: 100 }).unwrap();
    handle2
        .send(StringMessage {
            value: "Hello World".into(),
        })
        .unwrap();

    let msg: NumberMessage = listener1.recv().await.unwrap();
    assert_eq!(msg.value, 100);
    let msg: StringMessage = listener2.recv().await.unwrap();
    assert_eq!(msg.value, "Hello World");
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_anycast_local_two_registration_allowed() {
    use tokio::select;

    let mb1 = msgbus::MsgBus::new();
    let mut handle1 = mb1.handle().clone();
    let mut listener1 = handle1.register_anycast::<NumberMessage>().await.unwrap();
    let mut handle2 = mb1.handle().clone();
    let mut listener2 = handle2.register_anycast::<NumberMessage>().await.unwrap();
    handle1.send(NumberMessage { value: 100 }).unwrap();

    let msg = select! {
        msg1 = listener1.recv() => msg1,
        msg2 = listener2.recv() => msg2,
    }
    .unwrap();
    assert_eq!(msg.value, 100);
}

#[cfg(feature = "tokio")]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_rpc_local() {
    // tracing_subscriber::fmt::init();
    // console_subscriber::init();
    use msgbus::spawn;

    let mb1 = msgbus::MsgBus::new();
    let mut handle1 = mb1.handle().clone();
    // dbg!("Before Spawn: {:?}", &handle1);
    let mut listener1 = handle1.register_rpc().await.unwrap();

    spawn(async move {
        // dbg!("Registered rpc: {:?}", &listener1);

        let mut request = listener1.recv().await.unwrap();
        // dbg!("Got request: {:?}", &request);
        let msg: RpcMessage = request.payload().take().unwrap();
        let response = RpcResponse {
            value: (msg.value as i32) * 20,
        };
        request.respond(response).unwrap();
    });
    // dbg!("MsgBus: {:?}", &mb1);

    let handle2 = mb1.handle().clone();
    let response = handle2.request(RpcMessage { value: 5 }).await.unwrap();

    assert_eq!(response.value, 100);
    drop(handle1);
    drop(handle2);
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_multicast_local_one_listener() {
    let mb1 = msgbus::MsgBus::new();
    let mut handle1 = mb1.handle().clone();
    let handle2 = mb1.handle().clone();
    let mut listener1 = handle1.register_multicast::<NumberMessage>().await.unwrap();
    handle2.send(NumberMessage { value: 100 }).unwrap();
    let msg: NumberMessage = listener1.recv().await.unwrap();
    assert_eq!(msg.value, 100);
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_multicast_local_two_listener() {
    use tracing::debug;

    // console_subscriber::init();

    let mb1 = msgbus::MsgBus::new();
    let mut handle1 = mb1.handle().clone();
    let mut handle2 = mb1.handle().clone();
    let handle3 = mb1.handle().clone();
    let mut listener1 = handle1.register_multicast::<NumberMessage>().await.unwrap();
    let mut listener2 = handle2.register_multicast::<NumberMessage>().await.unwrap();
    debug!("Sending message");
    handle3.send(NumberMessage { value: 100 }).unwrap();
    debug!("Sent message");
    debug!("Listener1: {:#?}", listener1);
    let msg: NumberMessage = listener2.recv().await.unwrap();
    debug!("Received first message {:?}", msg);

    assert_eq!(msg.value, 100);
    let msg: NumberMessage = listener1.recv().await.unwrap();
    assert_eq!(msg.value, 100);
}
