mod common;
use crate::common::{NumberMessage, setup};

#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_unicast_two_buses() {
    tracing_subscriber::fmt::init();

    use tokio::time;

    let mb1 = msgbus::MsgBus::new();
    let mb2 = msgbus::MsgBus::new();
    let mut handle1 = mb1.handle().clone();
    let handle2 = mb2.handle().clone();
    let mut listener1 = handle1.register_unicast::<NumberMessage>().await.unwrap();
    time::sleep(std::time::Duration::from_secs(1)).await;
    handle2.send(NumberMessage { value: 100 }).unwrap();
    let msg: NumberMessage = listener1.recv().await.unwrap();
    assert_eq!(msg.value, 100);
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_unicast_three_buses_with_drop() {
    tracing_subscriber::fmt::init();

    use tokio::time;

    let mb1 = msgbus::MsgBus::new();
    let mb2 = msgbus::MsgBus::new();
    let mut handle1 = mb1.handle().clone();
    let handle2 = mb2.handle().clone();
    let mut listener1 = handle1.register_unicast::<NumberMessage>().await.unwrap();
    time::sleep(std::time::Duration::from_millis(10)).await;
    handle2.send(NumberMessage { value: 100 }).unwrap();

    let msg: NumberMessage = listener1.recv().await.unwrap();
    assert_eq!(msg.value, 100);
    drop(mb1);
    drop(handle1);
    drop(listener1);
    time::sleep(std::time::Duration::from_millis(10)).await;

    let mb3 = msgbus::MsgBus::new();
    let mut handle3 = mb3.handle().clone();
    let mut listener3 = handle3.register_unicast::<NumberMessage>().await.unwrap();
    time::sleep(std::time::Duration::from_millis(10)).await;

    handle2.send(NumberMessage { value: 200 }).unwrap();
    let msg = listener3.recv().await.unwrap();
    assert_eq!(msg.value, 200);
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn test_rpc_two_busses() {
    use tokio::{task::yield_now, time};

    tracing_subscriber::fmt::init();
    let mb1 = msgbus::MsgBus::new();
    let mb2 = msgbus::MsgBus::new();
    let mut handle1 = mb1.handle().clone();
    let handle2 = mb2.handle().clone();
    let mut listener1 = handle1.register_rpc::<common::RpcMessage>().await.unwrap();
    tokio::spawn(async move {
        let mut responder = listener1.recv().await.unwrap();
        let msg = responder.payload().unwrap();
        assert_eq!(msg.value, 42);
        time::sleep(std::time::Duration::from_millis(1000)).await;

        responder.reply(common::RpcResponse { value: 123 }).unwrap();
    });
    time::sleep(std::time::Duration::from_millis(1000)).await;

    let response = handle2
        .request(common::RpcMessage { value: 42 })
        .await
        .unwrap();
    assert_eq!(response.value, 123);
}
