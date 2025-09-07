use msgbus::BusRiderRpc;
use msgbus::bus_uuid;
use serde::{Deserialize, Serialize};
// use std::time::Duration;

// use tokio::time::timeout;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[bus_uuid("123e4567-e89b-12d3-a456-426614174000")]
pub struct NumberMessage {
    pub value: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[bus_uuid("123e4567-e89b-12d3-a456-426614174001")]
pub struct StringMessage {
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[bus_uuid("123e4567-e89b-12d3-a456-426614174002")]
pub struct RpcMessage {
    pub value: u8,
}

impl BusRiderRpc for RpcMessage {
    type Response = RpcResponse;
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RpcResponse {
    pub value: i32,
}
