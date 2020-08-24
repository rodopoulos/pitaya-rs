use crate::settings;
use async_trait::async_trait;
use pitaya_core::{
    cluster::{Error, Rpc, RpcServer, ServerInfo},
    metrics::{self},
    protos, utils,
};
use prost::Message;
use slog::{debug, error, info, o, trace, warn};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, oneshot, RwLock};

#[derive(Clone)]
pub struct NatsRpcServer {
    settings: Arc<settings::Nats>,
    connection: Arc<RwLock<Option<(nats::Connection, nats::subscription::Handler)>>>,
    this_server: Arc<ServerInfo>,
    runtime_handle: tokio::runtime::Handle,
    logger: slog::Logger,
    reporter: metrics::ThreadSafeReporter,
}

impl NatsRpcServer {
    pub fn new(
        logger: slog::Logger,
        this_server: Arc<ServerInfo>,
        settings: Arc<settings::Nats>,
        runtime_handle: tokio::runtime::Handle,
        reporter: metrics::ThreadSafeReporter,
    ) -> Self {
        Self {
            settings,
            this_server,
            logger,
            connection: Arc::new(RwLock::new(None)),
            runtime_handle,
            reporter,
        }
    }

    fn on_nats_message(
        mut message: nats::Message,
        logger: &slog::Logger,
        sender: &mpsc::Sender<Rpc>,
        runtime_handle: tokio::runtime::Handle,
        conn: Arc<RwLock<Option<(nats::Connection, nats::subscription::Handler)>>>,
        reporter: &metrics::ThreadSafeReporter,
    ) -> std::io::Result<()> {
        debug!(logger, "received nats message"; "message" => %message);

        let rpc_start = Instant::now();
        let mut sender = sender.clone();
        let req: protos::Request = Message::decode(message.data.as_ref())?;
        let (responder, response_receiver) = oneshot::channel();

        let route = if let Some(msg) = req.msg.as_ref() {
            msg.route.to_string()
        } else {
            String::new()
        };

        let response_topic = match message.reply.take() {
            Some(topic) => topic,
            None => {
                error!(logger, "received empty response topic from nats message");
                return Ok(());
            }
        };

        match sender.try_send(Rpc::new(req, responder)) {
            Ok(_) => {
                // For the moment we are ignoring the handle returned by the task.
                // Worst case scenario we will have to kill the task in the middle of its processing
                // at the end of the program.
                let _ = {
                    let logger = logger.clone();
                    // runtime.spawn(async move {
                    trace!(logger, "spawning response receiver task");
                    let reporter = reporter.clone();
                    runtime_handle.spawn(async move {
                        match response_receiver.await {
                            Ok(response) => {
                                let conn = match conn.read().await.as_ref() {
                                    Some((conn, _)) => conn.clone(),
                                    _ => {
                                        error!(logger, "connection not open, cannot answer");
                                        return;
                                    }
                                };

                                debug!(logger, "responding rpc");
                                if let Err(err) = Self::respond(&conn, &response_topic, response)
                                {
                                    error!(logger, "failed to respond rpc"; "error" => %err);
                                    metrics::record_histogram_duration(reporter, "rpc_latency", rpc_start, &[&route, "failed"]).await;
                                } else {
                                    metrics::record_histogram_duration(reporter, "rpc_latency", rpc_start, &[&route, "ok"]).await;
                                }
                            }
                            Err(e) => {
                                // Errors happen here if the channel was closed before sending a message.
                                error!(logger, "failed to receive response from RPC"; "error" => %e);
                            }
                        }
                    })
                };
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                let _ = {
                    let logger = logger.clone();
                    let reporter = reporter.clone();
                    runtime_handle.spawn(async move {
                        warn!(logger, "channel is full, dropping request");
                        let conn = match conn.read().await.as_ref() {
                            Some((conn, _)) => conn.clone(),
                            _ => {
                                error!(logger, "connection not open, cannot answer");
                                return;
                            }
                        };

                        let response = protos::Response {
                            error: Some(protos::Error {
                                code: "PIT-503".to_string(),
                                msg: "server is overloaded".to_string(),
                                ..Default::default()
                            }),
                            ..Default::default()
                        };
                        if let Err(err) = Self::respond(&conn, &response_topic, response) {
                            error!(logger, "failed to respond rpc"; "error" => %err);
                        }
                        metrics::record_histogram_duration(
                            reporter,
                            "rpc_latency",
                            rpc_start,
                            &[&route, "failed"],
                        )
                        .await;
                    })
                };
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!(logger, "rpc channel stoped being listened");
            }
        };

        Ok(())
    }

    fn respond(
        connection: &nats::Connection,
        reply_topic: &str,
        res: protos::Response,
    ) -> Result<(), Error> {
        let buffer = utils::encode_proto(&res);
        connection.publish(reply_topic, buffer).map_err(Error::Nats)
    }

    async fn register_metrics(&self) {
        self.reporter
            .write()
            .await
            .register_histogram(metrics::Opts {
                namespace: String::from("pitaya"),
                subsystem: String::from("nats_rpc_server"),
                name: String::from("rpc_latency"),
                help: String::from("histogram of rpc latency in seconds"),
                variable_labels: vec!["route".to_string(), "status".to_string()],
                buckets: metrics::exponential_buckets(0.0005, 2.0, 20),
            })
            .expect("should not fail to register");
    }
}

#[async_trait]
impl RpcServer for NatsRpcServer {
    // Starts the server.
    async fn start(&mut self) -> Result<mpsc::Receiver<Rpc>, Error> {
        // Register relevant metrics.
        self.register_metrics().await;

        if self.connection.read().await.is_some() {
            warn!(self.logger, "nats rpc server was already started!");
            return Err(Error::RpcServerAlreadyStarted);
        }

        // TODO(lhahn): add callbacks here for sending metrics.
        info!(self.logger, "server connecting to nats"; "url" => &self.settings.url);
        let nats_connection = nats::ConnectionOptions::new()
            .max_reconnects(Some(self.settings.max_reconnection_attempts as usize))
            .connect(&self.settings.url)
            .map_err(Error::Nats)?;

        let (rpc_sender, rpc_receiver) = mpsc::channel(self.settings.max_rpcs_queued as usize);

        let sub = {
            let topic = utils::topic_for_server(&self.this_server);
            let logger = self.logger.new(o!());

            info!(self.logger, "rpc server subscribing"; "topic" => &topic);

            let sender = rpc_sender;
            let runtime_handle = self.runtime_handle.clone();
            let connection = self.connection.clone();
            let reporter = self.reporter.clone();
            nats_connection
                .subscribe(&topic)
                .map_err(Error::Nats)?
                .with_handler(move |message| {
                    Self::on_nats_message(
                        message,
                        &logger,
                        &sender,
                        runtime_handle.clone(),
                        connection.clone(),
                        &reporter,
                    )
                })
        };

        self.connection
            .write()
            .await
            .replace((nats_connection, sub));
        Ok(rpc_receiver)
    }

    // Shuts down the server.
    async fn shutdown(&mut self) -> Result<(), Error> {
        if let Some((connection, sub_handler)) = self.connection.write().await.take() {
            sub_handler.unsubscribe().map_err(Error::Nats)?;
            connection.close();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NatsRpcClient;
    use pitaya_core::{
        cluster::{RpcClient, ServerId, ServerKind},
        context, message,
    };
    use std::collections::HashMap;
    use std::error::Error as StdError;

    #[tokio::test]
    async fn server_starts_and_stops() -> Result<(), Box<dyn StdError>> {
        let sv = Arc::new(ServerInfo {
            id: ServerId::from("my-id"),
            kind: ServerKind::from("room"),
            metadata: HashMap::new(),
            frontend: false,
            hostname: "".to_owned(),
        });

        let mut rpc_server = NatsRpcServer::new(
            test_helpers::get_root_logger(),
            sv.clone(),
            Arc::new(Default::default()),
            tokio::runtime::Handle::current(),
            Arc::new(RwLock::new(Box::new(metrics::DummyReporter {}))),
        );
        let mut rpc_server_conn = rpc_server.start().await?;

        let handle = {
            tokio::spawn(async move {
                loop {
                    if let Some(rpc) = rpc_server_conn.recv().await {
                        let res = protos::Response {
                            data: "HEY, THIS IS THE SERVER".as_bytes().to_owned(),
                            error: None,
                        };
                        if let Err(_e) = rpc.responder().send(res) {
                            panic!("failed to respond rpc");
                        }
                    } else {
                        break;
                    }
                }
            })
        };

        {
            let mut client = NatsRpcClient::new(
                test_helpers::get_root_logger(),
                Arc::new(Default::default()),
                sv.clone(),
            );
            client.start().await?;

            let res = client
                .call(
                    context::Context::new(),
                    protos::RpcType::User,
                    message::Message {
                        kind: message::Kind::Request,
                        id: 12,
                        data: "sending some data".as_bytes().to_owned(),
                        route: "room.room.join".to_owned(),
                        compressed: false,
                        err: false,
                    },
                    sv.clone(),
                )
                .await?;

            assert_eq!(
                String::from_utf8_lossy(&res.data),
                "HEY, THIS IS THE SERVER"
            );
            client.shutdown().await?;
        }

        rpc_server.shutdown().await?;
        handle.await?;
        Ok(())
    }
}