#[cfg(feature = "ws_server")]
use tokio::sync::mpsc::Sender;
use tracing::error;

#[cfg(feature = "ws_server")]
use crate::peers::ws::StreamDirection;
use crate::{
    anybus::config::WebSocketServerConfig,
    peers::ws::{WsCommand, WsError},
    spawn,
};

use std::{fs::File, io::BufReader};

pub(super) async fn create_listener(
    ws_listener_options: WebSocketServerConfig,
    ws_command: tokio::sync::mpsc::Sender<WsCommand>,
) -> Result<(), WsError> {
    // Create the listener here
    //
    //
    let (cert_path, key_path) = (
        ws_listener_options.cert_path.unwrap_or_default(),
        ws_listener_options.key_path.unwrap_or_default(),
    );

    // let notls_acceptor = {

    // }

    // let acceptor = {
    //     let cert = std::fs::read(cert_path)?; //.expect("Failed to read certificate");
    //     let key = std::fs::read(key_path)?; //.expect("Failed to read private key");
    //     let identity = Identity::from_pkcs8(&cert, &key)?; // .expect("Failed to create identity from pkcs8");
    //     let acceptor = tokio_native_tls::TlsAcceptor::from(
    //         tokio_native_tls::native_tls::TlsAcceptor::builder(identity).build()?,
    //         // .expect("Failed to build TlsAcceptor"),
    //     );
    //     acceptor
    // };

    let acceptor = if !ws_listener_options.enable_tls {
        None
    } else {
        use std::sync::Arc;

        use rustls::pki_types::pem::PemObject;
        use rustls_pemfile::certs;
        use tokio_rustls::rustls::pki_types::PrivateKeyDer;

        // let cert = std::fs::read(cert_path)?; //.expect("Failed to read certificate");
        // let key = std::fs::read(key_path)?; //.expect("Failed to read private key");
        // // let certs = CertificateDer::from_pem_file(file_name)
        let cert_file = &mut BufReader::new(
            File::open(&cert_path)
                .map_err(|e| e.to_string())
                .expect("Failed to open certificate file {cert_path}"),
        );
        // let key_file =
        //     &mut BufReader::new(File::open(&key_path).map_err(|e| e.to_string()).unwrap());
        let certs = certs(cert_file).filter_map(|c| c.ok()).collect();

        // let cert = CertificateDer::from_pem_file(&cert_path)?;

        let key = PrivateKeyDer::from_pem_file(&key_path)?;
        // let certs = vec![cert];
        // let key = key.
        // let key = PrivateKeyDer::Pkcs8(key);
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));

        // let identity = Identity::from_pkcs8(&cert, &key)?; // .expect("Failed to create identity from pkcs8");
        // let acceptor = tokio_native_tls::TlsAcceptor::from(
        //     tokio_native_tls::native_tls::TlsAcceptor::builder(identity).build()?,
        //     // .expect("Failed to build TlsAcceptor"),
        // );
        Some(acceptor)
    };
    let sock_addr =
        std::net::SocketAddr::new(ws_listener_options.address, ws_listener_options.port);
    let listener = tokio::net::TcpListener::bind(sock_addr)
        .await
        .map_err(|e| {
            error!("Failed to bind to address {}: {}", sock_addr, e);
            WsError::BindFailure(sock_addr)
        })?;
    spawn(run_ws_listener(listener, ws_command, acceptor));
    Ok(())
}

#[cfg(feature = "ws_server")]
async fn run_ws_listener(
    listener: tokio::net::TcpListener,
    ws_command: Sender<WsCommand>,
    acceptor: Option<tokio_rustls::TlsAcceptor>,
) {
    loop {
        // use tokio_tungstenite::MaybeTlsStream;

        use tokio_tungstenite::MaybeTlsStream;

        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, addr)) => {
                        // Handle the new connection
                        let command = match acceptor {
                            Some(ref a) => match a.accept(stream).await {
                                Ok(s) => {
                                    // let s = rustls::client::TlsStream::fr
                                    // let s = MaybeTlsStream::RustlsClientServer(tokio_rustls::TlsStream::Server(s));
                                    let stream = match tokio_tungstenite::accept_async(s).await {
                                        Ok(stream) => stream,
                                        Err(e) => { tracing::error!("Failed to accept websocket connection from {}: {}", addr, e);
                                            continue },
                                    };
                                    WsCommand::NewWsStream(stream.into(),addr, StreamDirection::Inbound)
                                }
                                ,
                                Err(e) => {
                                    error!("TLS handshake failed with {}: {}", addr, e);
                                    continue;
                                }
                            },
                            None => {
                                let stream = tokio_tungstenite::accept_async(MaybeTlsStream::Plain(stream )).await.unwrap();
                                    WsCommand::NewWsStream(stream.into(),addr, StreamDirection::Inbound)
                                }
                        };
                        // let s = stream..into_inner();
                        // let stream = match stream {
                        //     Ok(s) => {
                        //         // let s = rustls::client::TlsStream::fr
                        //         // let s = MaybeTlsStream::RustlsClientServer(tokio_rustls::TlsStream::Server(s));
                        //         tokio_tungstenite::accept_async(s).await.unwrap()}
                        //     ,
                        //     Err(e) => {
                        //         error!("TLS handshake failed with {}: {}", addr, e);
                        //         continue;
                        //     }
                        // };
                        tracing::info!("Accepted connection from {}", addr);

                        ws_command.send(command).await.ok();
                    }
                    Err(e) => {
                        error!("Failed to accept connection: {}", e);
                    }
                }
            }

        }
    }
}
