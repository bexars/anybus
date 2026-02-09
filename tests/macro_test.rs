#[cfg(test)]
mod macro_test {
    use anybus::anybus_rpc;

    use anybus::{AnyBus, errors::AnyBusHandleError};
    use serde::Serialize;
    use uuid::Uuid;

    pub trait TestTrait {
        async fn greet(&mut self, name: String) -> String;
    }

    #[derive(Debug, Clone, Serialize)]
    struct TestImpl;

    #[anybus_rpc(uuid = "12345678_1234_1234_1234_123456789")]
    impl TestTrait for TestImpl {
        async fn greet(&mut self, name: String) -> String {
            format!("Hello, {}!", name)
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

    pub trait MathTrait {
        async fn add(&mut self, a: i32, b: i32) -> i32;
        async fn multiply(&mut self, a: i32, b: i32) -> i32;
    }

    #[derive(Debug, Clone, Serialize)]
    struct MathImpl;

    #[anybus_rpc]
    impl MathTrait for MathImpl {
        async fn add(&mut self, a: i32, b: i32) -> i32 {
            a + b
        }
        async fn multiply(&mut self, a: i32, b: i32) -> i32 {
            a * b
        }
    }

    #[tokio::test]
    async fn test_macro_math() {
        let bus = AnyBus::build().run();
        let handle = bus.handle().clone();
        let id = Uuid::now_v7();
        bus.add_bus_depot(MathImpl {}, id.clone().into());

        let helper = handle.rpc_helper().await.unwrap();
        let mut client = MathTraitClient::new_with_uuid(helper, id);
        let res_add = client.add(2, 3).await.unwrap();
        assert_eq!(res_add, 5);
        let res_mul = client.multiply(4, 5).await.unwrap();
        assert_eq!(res_mul, 20);
    }
}