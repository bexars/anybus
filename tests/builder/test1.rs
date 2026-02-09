use anybus::{BusRiderRpc, bus_uuid};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[bus_uuid("123e4567-e89b-12d3-a456-426614174000")]
pub struct NumberMessage {
    pub value: u32,
}

impl BusRiderRpc for NumberMessage {
    type Response = NumberMessage;
}
// Test to make sure that the builder doesn't build in certain configurations
fn main() {
    use anybus::AnyBusBuilder;

    let bus = AnyBusBuilder::new().init();
    bus.run();
    let handle = bus.handle().clone();
    handle
        .listener()
        .broadcast()
        .rpc()
        .register::<NumberMessage>();
    handle
        .listener()
        .rpc()
        .broadcast()
        .register::<NumberMessage>();
    handle
        .listener()
        .anycast()
        .rpc()
        .register::<NumberMessage>();
    handle
        .listener()
        .rpc()
        .anycast()
        .register::<NumberMessage>();
}
