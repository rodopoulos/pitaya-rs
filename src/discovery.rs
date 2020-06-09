use super::{Error, Server, ServerId, ServerKind};
use async_trait::async_trait;
use etcd_client::GetOptions;
use log::{debug, error, info};
use std::collections::HashMap;
use std::error::Error as StdError;
use std::future::Future;
use std::iter::FromIterator;
use std::sync::Arc;
use std::time::Duration;

trait Listener {
    fn server_added(&mut self, server: Server);
    fn server_removed(&mut self, server: Server);
}

#[async_trait]
trait ServiceDiscovery {
    async fn server_by_id(
        &mut self,
        id: &ServerId,
        kind: &ServerKind,
    ) -> Result<Option<Arc<Server>>, Error>;
    async fn servers_by_type(&mut self, sv_type: &ServerKind) -> Result<Vec<Arc<Server>>, Error>;

    fn add_listener(&mut self, _listener: Box<dyn Listener>) {}
    fn remove_listener(&mut self, _listener: Box<dyn Listener>) {}
}

// This service discovery is a lazy implementation.
struct EtcdLazy {
    client: etcd_client::Client,
    prefix: String,
    this_server: Server,
    lease_id: Option<i64>,
    listeners: Vec<Box<dyn Listener + Send>>,
    keep_alive_task: Option<(
        tokio::task::JoinHandle<()>,
        tokio::sync::oneshot::Sender<()>,
    )>,
    app_die_sender: tokio::sync::oneshot::Sender<()>,
    lease_ttl: Duration,

    // TODO: Shouldn't this fields be mutexes?
    servers_by_id: HashMap<ServerId, Arc<Server>>,
    servers_by_type: HashMap<ServerKind, HashMap<ServerId, Arc<Server>>>,
}

impl EtcdLazy {
    pub(crate) async fn new(
        prefix: String,
        server: Server,
        url: &str,
        lease_ttl: Duration,
        app_die_sender: tokio::sync::oneshot::Sender<()>,
    ) -> Result<Self, etcd_client::Error> {
        let client = etcd_client::Client::connect([url], None).await?;
        Ok(Self {
            client: client,
            prefix: prefix,
            this_server: server,
            servers_by_id: HashMap::new(),
            servers_by_type: HashMap::new(),
            lease_id: None,
            listeners: Vec::new(),
            keep_alive_task: None,
            lease_ttl: lease_ttl,
            app_die_sender: app_die_sender,
        })
    }

    pub(crate) async fn start(&mut self) -> Result<(), Error> {
        self.grant_lease().await?;
        self.add_server_to_etcd().await?;
        self.start_watch().await?;
        Ok(())
    }

    pub(crate) async fn stop(&mut self) -> Result<(), Error> {
        if let Some((handle, sender)) = self.keep_alive_task.take() {
            info!("stopping etcd service discovery");
            sender.send(()).map_err(|_| {
                error!("failed to send stop message");
            });
            handle.await?;
        }
        Ok(())
    }

    fn server_kind_prefix(&self, server_kind: &ServerKind) -> String {
        format!("{}/servers/{}/", self.prefix, server_kind.0)
    }

    async fn cache_server_kind(&mut self, server_kind: &ServerKind) -> Result<(), Error> {
        info!(
            "server id not found in cache, filling cache for kind {}",
            server_kind.0
        );
        let resp = {
            let key_prefix = self.server_kind_prefix(server_kind);
            self.client
                .get(key_prefix, Some(GetOptions::new().with_prefix()))
                .await?
        };
        info!("etcd returned {} keys", resp.kvs().len());
        for kv in resp.kvs() {
            let server_str = kv.value_str()?;
            println!("server string: {}", server_str);
            let new_server: Arc<Server> = Arc::new(serde_json::from_str(server_str)?);
            let new_server_id = new_server.id.clone();

            self.servers_by_id
                .insert(new_server.id.clone(), new_server.clone());

            self.servers_by_type
                .entry(new_server.kind.clone())
                .and_modify(|servers| {
                    servers.insert(new_server_id.clone(), new_server.clone());
                })
                .or_insert(HashMap::from_iter(
                    [(new_server_id, new_server)].iter().cloned(),
                ));
        }
        Ok(())
    }

    async fn lease_keep_alive(
        mut lease_ttl: Duration,
        keeper: etcd_client::LeaseKeeper,
        stream: etcd_client::LeaseKeepAliveStream,
        mut stop_chan: tokio::sync::oneshot::Receiver<()>,
        app_die_chan: tokio::sync::oneshot::Sender<()>,
    ) {
        use tokio::time::timeout;

        info!("keep alive task started");
        loop {
            let seconds_to_wait = lease_ttl.as_secs() as f32 - (lease_ttl.as_secs() as f32 / 3.0);
            assert!(seconds_to_wait > 0.0);

            match timeout(Duration::from_secs(seconds_to_wait as u64), &mut stop_chan).await {
                Err(_) => {
                    // TODO(lhahn): currently, the ttl will fail as soon as it loses connection to etcd.
                    // Figure out if a more robust retrying scheme is necessary here.
                    if let Err(e) = keeper.keep_alive().await {
                        error!("failed keep alive request: {}", e);
                        if let Err(_) = app_die_chan.send(()) {
                            error!("failed to send die message");
                        }
                        return;
                    }
                    match stream.message().await {
                        Err(_) => {
                            error!("failed to get keep alive response: {}", e);
                        }
                        Ok(msg) => {
                            if let Some(response) = msg {
                                debug!("lease renewed with new ttl of {} seconds", response.ttl());
                                assert!(response.ttl() > 0);
                                lease_ttl = Duration::from_secs(response.ttl() as u64);
                            } else {
                                // TODO(lhahn): what to do here?
                            }
                        }
                    }
                }
                Ok(_) => {
                    info!("received stop message, exiting lease keep alive task");
                    return;
                }
            }
        }
    }

    async fn grant_lease(&mut self) -> Result<(), Error> {
        assert!(self.lease_id.is_none());
        assert!(self.keep_alive_task.is_none());

        let lease_ttl = std::time::Duration::from_secs(4);
        let lease_response = self
            .client
            .lease_grant(lease_ttl.as_secs() as i64, None)
            .await?;
        self.lease_id = Some(lease_response.id());

        let (keeper, stream) = self.client.lease_keep_alive(lease_response.id()).await?;
        let (stop_sender, stop_receiver) = tokio::sync::oneshot::channel::<()>();
        let (app_die_sender, app_die_receiver) = tokio::sync::oneshot::channel::<()>();

        self.keep_alive_task = Some((
            tokio::spawn(Self::lease_keep_alive(
                self.lease_ttl.clone(),
                keeper,
                stream,
                stop_receiver,
                app_die_sender,
            )),
            stop_sender,
        ));

        Ok(())
    }

    fn get_etcd_server_key(&self) -> String {
        format!(
            "{}/servers/{}/{}",
            self.prefix, self.this_server.kind.0, self.this_server.id.0
        )
    }

    async fn add_server_to_etcd(&mut self) -> Result<(), Error> {
        assert!(self.lease_id.is_some());
        let key = self.get_etcd_server_key();
        let server_json = serde_json::to_vec(&self.this_server)?;
        if let Some(lease_id) = self.lease_id {
            let options = etcd_client::PutOptions::new().with_lease(lease_id);
            self.client.put(key, server_json, Some(options)).await?;
        } else {
            unreachable!();
        }
        Ok(())
    }

    async fn watch_task(watcher: etcd_client::Watcher, stream: etcd_client::WatchStream) {
        use tokio::time::timeout;

        unimplemented!()
    }

    async fn start_watch(&mut self) -> Result<(), Error> {
        let watch_prefix = format!("{}/servers/", self.prefix);
        let options = etcd_client::WatchOptions::new().with_prefix();
        let (watcher, watch_stream) = self.client.watch(watch_prefix, Some(options)).await?;

        let handle = tokio::spawn(Self::watch_task(watcher, watch_stream));

        Ok(())
    }
}

#[async_trait]
impl ServiceDiscovery for EtcdLazy {
    async fn server_by_id(
        &mut self,
        server_id: &ServerId,
        server_kind: &ServerKind,
    ) -> Result<Option<Arc<Server>>, Error> {
        if let Some(server) = self.servers_by_id.get(server_id).map(|sv| sv.clone()) {
            return Ok(Some(server));
        }

        info!(
            "server id not found in cache, filling cache for kind {}",
            server_kind.0
        );
        let resp = {
            let key_prefix = self.server_kind_prefix(server_kind);
            self.client
                .get(key_prefix, Some(GetOptions::new().with_prefix()))
                .await?
        };
        info!("etcd returned {} keys", resp.kvs().len());
        self.cache_server_kind(server_kind).await?;

        Ok(self.servers_by_id.get(server_id).map(|sv| sv.clone()))
    }

    async fn servers_by_type(&mut self, _server: &ServerKind) -> Result<Vec<Arc<Server>>, Error> {
        unimplemented!()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const ETCD_URL: &str = "localhost:2379";
    const INVALID_ETCD_URL: &str = "localhost:1234";

    fn new_server() -> Server {
        Server {
            frontend: true,
            hostname: "".to_owned(),
            id: ServerId::new(),
            kind: ServerKind::new(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn sd_can_be_create() -> Result<(), Box<dyn StdError>> {
        let mut rt = tokio::runtime::Runtime::new()?;
        let server = new_server();
        let (app_die_sender, _app_die_recv) = tokio::sync::oneshot::channel();
        let _sd = rt.block_on(async move {
            EtcdLazy::new(
                "pitaya".to_owned(),
                server,
                ETCD_URL,
                Duration::from_secs(60),
                app_die_sender,
            )
            .await
        })?;
        Ok(())
    }

    #[test]
    #[should_panic]
    fn sd_can_fail_creation() {
        let mut rt = tokio::runtime::Runtime::new().unwrap();
        let server = new_server();
        let (app_die_sender, _app_die_recv) = tokio::sync::oneshot::channel();
        let _sd = rt
            .block_on(async move {
                EtcdLazy::new(
                    "pitaya".to_owned(),
                    server,
                    INVALID_ETCD_URL,
                    Duration::from_secs(60),
                    app_die_sender,
                )
                .await
            })
            .unwrap();
    }

    #[test]
    fn cache_empty_on_start() -> Result<(), Box<dyn StdError>> {
        let mut rt = tokio::runtime::Runtime::new().unwrap();
        let server = new_server();
        let (app_die_sender, _app_die_recv) = tokio::sync::oneshot::channel();
        let sd = rt.block_on(async move {
            EtcdLazy::new(
                "pitaya".to_owned(),
                server,
                ETCD_URL,
                Duration::from_secs(60),
                app_die_sender,
            )
            .await
        })?;
        assert_eq!(sd.servers_by_id.len(), 0);
        assert_eq!(sd.servers_by_type.len(), 0);
        Ok(())
    }

    async fn server_by_id_main(
        server_id: &ServerId,
    ) -> Result<(EtcdLazy, Option<Arc<Server>>), Box<dyn StdError>> {
        let server = new_server();
        let (app_die_sender, _app_die_recv) = tokio::sync::oneshot::channel();
        let mut sd = EtcdLazy::new(
            "pitaya".to_owned(),
            server,
            ETCD_URL,
            Duration::from_secs(60),
            app_die_sender,
        )
        .await?;
        let maybe_server = sd
            .server_by_id(server_id, &ServerKind::from("room"))
            .await?;
        Ok((sd, maybe_server))
    }

    #[test]
    fn server_by_id_works() -> Result<(), Box<dyn StdError>> {
        let mut rt = tokio::runtime::Runtime::new().unwrap();
        let (sd, server) = rt.block_on(server_by_id_main(&ServerId::from("random-id")))?;
        assert!(server.is_none());
        assert_eq!(sd.servers_by_id.len(), 1);

        let mut server_id: Option<ServerId> = None;
        for (id, _) in sd.servers_by_id.iter() {
            server_id = Some(id.clone());
        }

        let (sd, server) = rt.block_on(server_by_id_main(server_id.as_ref().unwrap()))?;

        assert!(server.is_some());
        assert_eq!(sd.servers_by_id.len(), 1);
        assert_eq!(sd.servers_by_type.len(), 1);
        assert_eq!(
            sd.servers_by_type
                .get(&ServerKind::from("room"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(server_id.unwrap(), server.unwrap().id);
        Ok(())
    }

    #[test]
    fn server_by_type_works() -> Result<(), Box<dyn StdError>> {
        unimplemented!()
    }

    #[test]
    fn server_lease_works() -> Result<(), Box<dyn StdError>> {
        unimplemented!()
    }

    #[test]
    fn server_watch_works() -> Result<(), Box<dyn StdError>> {
        unimplemented!()
    }
}
