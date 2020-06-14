use crate::{error::Error, protos, utils, Server};
use async_trait::async_trait;
use log::{debug, info, warn};
use std::sync::Arc;

#[async_trait]
pub(crate) trait RpcServer {
    async fn recv(&mut self) -> Result<protos::Response, Error>;
}

pub(crate) struct NatsRpcServer {
    address: String,
    connection: Option<(nats::Connection, nats::subscription::Handler)>,
    max_reconnects: usize,
    this_server: Arc<Server>,
}

impl NatsRpcServer {
    pub(crate) fn new(this_server: Arc<Server>, address: String, max_reconnects: usize) -> Self {
        Self {
            address: address,
            connection: None,
            max_reconnects: max_reconnects,
            this_server: this_server,
        }
    }

    pub(crate) fn start(&mut self) -> Result<(), Error> {
        if let Some(_) = self.connection {
            warn!("nats rpc server was already started!");
            return Ok(());
        }

        // TODO(lhahn): add callbacks here for sending metrics.
        let connection = nats::ConnectionOptions::new()
            .max_reconnects(Some(self.max_reconnects))
            .connect(&self.address)
            .map_err(|e| Error::Nats(e))?;

        let topic = utils::topic_for_server(&self.this_server);
        info!("rpc server subscribing on topic {}", topic);

        let sub = connection
            .subscribe(&topic)
            .map_err(|e| Error::Nats(e))?
            .with_handler(move |msg| {
                info!("received msg: {}", &msg);
                Ok(())
            });

        self.connection = Some((connection, sub));
        Ok(())
    }

    pub fn close(&mut self) -> Result<(), Error> {
        if let Some((connection, sub_handler)) = self.connection.take() {
            sub_handler.unsubscribe().map_err(|e| Error::Nats(e))?;
            connection.close();
        }
        Ok(())
    }
}
