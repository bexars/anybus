#[cfg(test)]
mod macro_test {
    // use anybus::anybus_rpc;

    use anybus::{AnyBus, errors::AnyBusHandleError};
    use serde::Serialize;
    use uuid::Uuid;

    pub trait TestTrait {
        async fn greet(&mut self, name: String) -> String;
    }

    #[derive(Debug, Clone, Serialize)]
    struct TestImpl;

    impl TestTrait for TestImpl {
        async fn greet(&mut self, name: String) -> String {
            format!("Hello, {}!", name)
        }
    }

    // Everything below this should be generated off of the trait, struct and impl above
    // Expanded/generated code from #[anybus_rpc]
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub enum TestTraitRequest {
        Greet { name: String },
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub enum TestTraitRpcResponse {
        Greet(String),
    }

    impl ::anybus::BusRiderRpc for TestTraitRequest {
        type Response = TestTraitRpcResponse;
    }

    // Expanded/generated code from #[anybus_rpc(uuid="12345678_1234_1234_1234_123456789")]
    // This should be optional in the macro creation
    impl ::anybus::BusRiderWithUuid for TestTraitRequest {
        const ANYBUS_UUID: uuid::Uuid =
            uuid::Uuid::from_u128(12345678_1234_1234_1234_123456789_u128); // Simulated UUID from macro
    }

    // Expanded/generated code from #[anybus_rpc(uuid="12345678_1234_1234_1234_123456789")]
    // This should be optional in the macro creation
    impl TestImpl {
        fn get_uuid() -> Uuid {
            uuid::Uuid::from_u128(12345678_1234_1234_1234_123456789_u128)
        }
    }

    #[derive(Debug)]
    pub struct TestTraitClient {
        rpc_helper: anybus::RequestHelper,
        endpoint_uuid: uuid::Uuid,
    }

    // #[async_trait::async_trait]
    impl TestTraitClient {
        async fn greet(&mut self, name: String) -> Result<String, AnyBusHandleError> {
            let request = TestTraitRequest::Greet { name };
            let response = self
                .rpc_helper
                .request_to_uuid(request, self.endpoint_uuid)
                .await;
            let response = match response {
                Ok(r) => r,
                Err(e) => return Err(e),
            };
            let response = match response {
                TestTraitRpcResponse::Greet(result) => result,
                _ => panic!("Unexpected response variant"),
            };
            Ok(response)
        }
    }

    impl TestTraitClient {
        pub fn new_with_uuid(
            rpc_helper: ::anybus::RequestHelper,
            endpoint_uuid: uuid::Uuid,
        ) -> Self {
            Self {
                rpc_helper,
                endpoint_uuid,
            }
        }
        pub fn new(rpc_helper: ::anybus::RequestHelper) -> Self {
            Self::new_with_uuid(
                rpc_helper,
                <TestTraitRequest as ::anybus::BusRiderWithUuid>::ANYBUS_UUID,
            )
        }
    }

    // #[async_trait::async_trait]
    impl ::anybus::BusDepot<TestTraitRequest> for TestImpl {
        async fn on_request(
            &mut self,
            request: Option<TestTraitRequest>,
            _handle: &::anybus::Handle,
        ) -> TestTraitRpcResponse {
            match request.unwrap() {
                TestTraitRequest::Greet { name } => {
                    let result = self.greet(name).await;
                    TestTraitRpcResponse::Greet(result)
                }
            }
        }
    }

    #[test]
    fn test_macro_compiles() {
        // Just check that the macro generates the code without errors
        // let _depot = TestTraitDepot { impl_: TestImpl };
        // Note: Actual runtime test would require setting up AnyBus, but this checks compilation
    }

    #[tokio::test]
    async fn test_macro_rpc() {
        let bus = AnyBus::build().run();
        let handle = bus.handle().clone();
        let id = Uuid::now_v7();
        bus.add_bus_depot(TestImpl {}, id.clone().into());

        let helper = handle.rpc_helper().await.unwrap();
        let mut client = TestTraitClient::new_with_uuid(helper, id);
        let res = client.greet("Matt".to_string()).await.unwrap();
        assert_eq!(res, "Hello, Matt!")
    }

    #[tokio::test]
    async fn test_macro_rpc_uuid() {
        let bus = AnyBus::build().run();
        let handle = bus.handle().clone();
        // let id = Uuid::now_v7();
        bus.add_bus_depot(TestImpl {}, TestImpl::get_uuid().into());

        let helper = handle.rpc_helper().await.unwrap();
        let mut client = TestTraitClient::new(helper);
        let res = client.greet("Matt".to_string()).await.unwrap();
        assert_eq!(res, "Hello, Matt!")
    }
}
