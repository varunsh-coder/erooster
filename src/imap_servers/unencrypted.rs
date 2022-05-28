use crate::{
    backend::database::DB,
    config::Config,
    imap_commands::Data,
    imap_servers::Server,
    imap_servers::{
        state::{Connection, State},
        CAPABILITY_HELLO,
    },
    line_codec::LinesCodec,
};
use async_trait::async_trait;
use futures::{channel::mpsc, SinkExt, StreamExt};
use notify::Event;
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{broadcast, RwLock},
};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_util::codec::Framed;
use tracing::{debug, info};

/// An unencrypted imap Server
pub struct Unencrypted;

#[async_trait]
impl Server for Unencrypted {
    async fn run(
        config: Arc<Config>,
        database: DB,
        mut _file_watcher: broadcast::Sender<Event>,
    ) -> color_eyre::eyre::Result<()> {
        let addr: Vec<SocketAddr> = if let Some(listen_ips) = &config.listen_ips {
            listen_ips
                .iter()
                .map(|ip| format!("{}:143", ip).parse().unwrap())
                .collect()
        } else {
            vec!["0.0.0.0:143".parse()?]
        };
        info!("[IMAP] Trying to listen on {:?}", addr);
        let listener = TcpListener::bind(&addr[..]).await?;
        info!("[IMAP] Listening on unecrypted Port");
        let mut stream = TcpListenerStream::new(listener);
        while let Some(Ok(tcp_stream)) = stream.next().await {
            let peer = tcp_stream.peer_addr().expect("peer addr to exist");
            debug!("[IMAP] Got new peer: {}", peer);

            let config = Arc::clone(&config);
            let database = Arc::clone(&database);
            tokio::spawn(async move {
                let lines = Framed::new(tcp_stream, LinesCodec::new());
                let (mut lines_sender, mut lines_reader) = lines.split();
                lines_sender
                    .send(CAPABILITY_HELLO.to_string())
                    .await
                    .unwrap();
                let state = Arc::new(RwLock::new(Connection {
                    state: State::NotAuthenticated,
                    secure: false,
                    username: None,
                }));

                let (mut tx, mut rx) = mpsc::unbounded();
                tokio::spawn(async move {
                    while let Some(res) = rx.next().await {
                        lines_sender.send(res).await.unwrap();
                    }
                });
                while let Some(Ok(line)) = lines_reader.next().await {
                    let data = Data {
                        con_state: Arc::clone(&state),
                    };

                    debug!("[IMAP] [{}] Got Command: {}", peer, line);

                    // TODO make sure to handle IDLE different as it needs us to stream lines
                    // TODO pass lines and make it possible to not need new lines in responds but instead directly use `lines.send`
                    let response = data
                        .parse(&mut tx, Arc::clone(&config), Arc::clone(&database), line)
                        .await;

                    match response {
                        Ok(response) => {
                            // Cleanup timeout managers
                            if response {
                                // Used for later session timer management
                                debug!("[IMAP] Closing connection");
                                break;
                            }
                        }
                        // We try a last time to do a graceful shutdown before closing
                        Err(e) => {
                            tx.send(format!("* BAD [SERVERBUG] This should not happen: {}", e))
                                .await
                                .unwrap();
                            debug!("[IMAP] Closing connection");
                            break;
                        }
                    }
                }
            });
        }
        Ok(())
    }
}
