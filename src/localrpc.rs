use std::time::Duration;
use tokio::sync::{
    mpsc::{self, Receiver},
    oneshot,
};

// The foundational trait linking requests to their specific return types
pub(crate) trait LocalRpcRequest {
    type Reply: Send + 'static;
}

/// A macro to create an RPC enum with associated request structs and return types.
/// This macro simplifies the creation of type-safe RPC calls by generating the necessary boilerplate code.
#[macro_export]
macro_rules! define_local_rpc {
    (
        $enum_name:ident {
            $( $variant:ident { $($field:ident : $ftype:ty),* $(,)? } -> $ret:ty ),* $(,)?
        }
    ) => {
        // 1. Generate individual structs for every request
        $(
            #[derive(Debug)]
            pub struct $variant { $(pub $field: $ftype),* }

            // 2. Link each struct to its variant-specific return type
            impl crate::localrpc::LocalRpcRequest for $variant {
                type Reply = $ret;
            }
        )*

        // 3. Generate the master enum for transport
        #[derive(Debug)]
        pub enum $enum_name {
            $(
                $variant {
                    req: $variant,
                    respond_to: tokio::sync::oneshot::Sender<$ret>,
                },
            )*
        }

        // 4. Implement From conversions automatically
        $(
            impl From<($variant, tokio::sync::oneshot::Sender<$ret>)> for $enum_name {
                fn from((req, respond_to): ($variant, tokio::sync::oneshot::Sender<$ret>)) -> Self {
                    $enum_name::$variant { req, respond_to }
                }
            }
        )*
    };
}

// Core operational framework errors
#[derive(Debug, thiserror::Error)] // using standard errors, or implement Debug/Display manually
pub(crate) enum RpcError {
    #[error("The remote task dropped the response channel")]
    ServerDropped,
    #[error("The remote task closed its main inbox channel")]
    ServerClosed,
    #[error("The RPC operation timed out after {}ms", .0.as_millis())]
    Timeout(Duration),
}

pub fn create_rpc<T>() -> (LocalRpcClient<T>, Receiver<T>) {
    let (tx, rx) = mpsc::channel(32);
    (LocalRpcClient { sender: tx }, rx)
}

#[derive(Clone, Debug)]
pub struct LocalRpcClient<T> {
    sender: mpsc::Sender<T>,
}

impl<T> LocalRpcClient<T> {
    pub async fn call<R>(&self, request: R) -> Result<R::Reply, RpcError>
    where
        R: LocalRpcRequest,
        T: From<(R, oneshot::Sender<R::Reply>)>,
    {
        self.call_with_timeout(request, Duration::from_millis(1000))
            .await
    }

    // Executes a type-safe request, returning a framework RpcError if execution stalls
    pub async fn call_with_timeout<R>(
        &self,
        request: R,
        limit: Duration,
    ) -> Result<R::Reply, RpcError>
    where
        R: LocalRpcRequest,
        T: From<(R, oneshot::Sender<R::Reply>)>,
    {
        let (tx, rx) = oneshot::channel();
        let message = T::from((request, tx));

        // Attempt to pass the request into the channel buffer
        self.sender
            .send(message)
            .await
            .map_err(|_| RpcError::ServerClosed)?;

        // Race the oneshot receiver against the tokio timer
        match tokio::time::timeout(limit, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_oneshot_dropped)) => Err(RpcError::ServerDropped),
            Err(_elapsed) => Err(RpcError::Timeout(limit)),
        }
    }
}
