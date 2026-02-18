#[cfg(test)]
mod macro_test {
    use anybus::anybus_rpc;

    use anybus::{AnyBus, BusRiderWithUuid, errors::AnyBusHandleError};
    use serde::Serialize;
    use uuid::Uuid;

    #[anybus_rpc(uuid = "12345678-1234-1234-1234-123456789abc")]
    pub trait TestTrait {
        async fn greet(&mut self, name: String) -> String;
    }

    #[derive(Debug, Clone, Serialize)]
    struct TestImpl;

    #[async_trait::async_trait]
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
        let depot: Box<dyn TestTrait + Send + Sync> = Box::new(TestImpl {});
        bus.add_bus_depot(depot, id.clone().into());

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
        let depot: Box<dyn TestTrait + Send + Sync> = Box::new(TestImpl {});
        bus.add_bus_depot(depot, TestTraitRequest::ANYBUS_UUID.into());

        let helper = handle.rpc_helper().await.unwrap();
        let mut client = TestTraitClient::new(helper);
        let res = client.greet("Matt".to_string()).await.unwrap();
        assert_eq!(res, "Hello, Matt!")
    }

    #[anybus_rpc]
    pub trait MathTrait {
        async fn add(&mut self, a: i32, b: i32) -> i32;
        async fn multiply(&mut self, a: i32, b: i32) -> i32;
    }

    #[derive(Debug, Clone, Serialize)]
    struct MathImpl;

    #[async_trait::async_trait]
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
        let depot: Box<dyn MathTrait + Send + Sync> = Box::new(MathImpl {});
        bus.add_bus_depot(depot, id.clone().into());

        let helper = handle.rpc_helper().await.unwrap();
        let mut client = MathTraitClient::new_with_uuid(helper, id);
        let res_add = client.add(2, 3).await.unwrap();
        assert_eq!(res_add, 5);
        let res_mul = client.multiply(4, 5).await.unwrap();
        assert_eq!(res_mul, 20);
    }

    #[anybus_rpc(uuid = "87654321-4321-4321-4321-cba987654321")]
    pub trait CalcTrait {
        async fn subtract(&mut self, a: i32, b: i32) -> i32;
        async fn divide(&mut self, a: i32, b: i32) -> i32;
    }

    #[derive(Debug, Clone)]
    struct CalcImpl;

    #[async_trait::async_trait]
    impl CalcTrait for CalcImpl {
        async fn subtract(&mut self, a: i32, b: i32) -> i32 {
            a - b
        }
        async fn divide(&mut self, a: i32, b: i32) -> i32 {
            a / b
        }
    }

    #[tokio::test]
    async fn test_macro_calc() {
        let bus = AnyBus::build().run();
        let handle = bus.handle().clone();
        let id = Uuid::now_v7();
        let depot: Box<dyn CalcTrait + Send + Sync> = Box::new(CalcImpl {});
        bus.add_bus_depot(depot, id.clone().into());

        let helper = handle.rpc_helper().await.unwrap();
        let mut client = CalcTraitClient::new_with_uuid(helper, id);
        let res_sub = client.subtract(10, 3).await.unwrap();
        assert_eq!(res_sub, 7);
        let res_div = client.divide(20, 4).await.unwrap();
        assert_eq!(res_div, 5);
    }
}
