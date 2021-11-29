//! RTR Clients.

use std::io;
use std::fs::File;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use futures::pin_mut;
use futures::future::{select, Either};
use log::{debug, error, info, warn};
use rpki::rtr::client::{Client, PayloadError, PayloadTarget, PayloadUpdate};
use rpki::rtr::payload::{Action, Payload, Timing};
use rpki::rtr::state::{Serial, State};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::{timeout_at, Instant};
use tokio_rustls::{
    TlsConnector, client::TlsStream, rustls::ClientConfig, webpki::DNSName,
    webpki::DNSNameRef
};
use crate::metrics;
use crate::comms::{Gate, GateMetrics, GateStatus, Terminated, UnitStatus};
use crate::manager::Component;
use crate::payload;

use std::pin::Pin;
use std::task::{Context, Poll};
use pin_project_lite::pin_project;
use tokio::io::{ReadBuf};

//------------ Tcp -----------------------------------------------------------

/// An RTR client using an unencrypted plain TCP socket.
#[derive(Debug, Deserialize)]
pub struct Tcp {
    /// The remote address to connect to.
    remote: String,

    /// How long to wait before connecting again if the connection is closed.
    #[serde(default = "Tcp::default_retry")]
    retry: u64,
}

impl Tcp {
    fn default_retry() -> u64 {
        60
    }

    pub async fn run(
        self, component: Component, gate: Gate
    ) -> Result<(), Terminated> {
        RtrClient::run(component, gate, self.retry, || {
            TcpStream::connect(&self.remote)
        }).await
    }
}


//------------ Tls -----------------------------------------------------------

/// An RTR client using an unencrypted plain TCP socket.
#[derive(Debug, Deserialize)]
pub struct Tls {
    /// The remote address to connect to.
    remote: String,

    /// How long to wait before connecting again if the connection is closed.
    #[serde(default = "Tcp::default_retry")]
    retry: u64,

    /// Paths to root certficates.
    #[serde(default)]
    cacerts: Vec<PathBuf>,
}

struct TlsState {
    tls: Tls,
    domain: DNSName,
    connector: TlsConnector,
}

impl Tls {
    pub async fn run(
        self, component: Component, gate: Gate
    ) -> Result<(), Terminated> {
        let domain = self.get_domain_name(component.name())?;
        let connector = self.build_connector(component.name())?;
        let retry = self.retry;
        let state = Arc::new(TlsState {
            tls: self, domain, connector
        });
        RtrClient::run(
            component, gate, retry,
            move || {
                Self::connect(state.clone())
            }
        ).await
    }

    fn get_domain_name(
        &self, unit_name: &str
    ) -> Result<DNSName, Terminated> {
        let host = if let Some((host, port)) = self.remote.rsplit_once(':') {
            if port.parse::<u16>().is_ok() {
                host
            }
            else {
                self.remote.as_ref()
            }
        }
        else {
            self.remote.as_ref()
        };
        DNSNameRef::try_from_ascii_str(host).map(Into::into).map_err(|err| {
            error!(
                "Unit {}: Invalid remote name '{}': {}'",
                unit_name, host, err
            );
            Terminated
        })
    }

    fn build_connector(
        &self, unit_name: &str
    ) -> Result<TlsConnector, Terminated> {
        let mut config = ClientConfig::new();
        config.root_store.add_server_trust_anchors(
            &webpki_roots::TLS_SERVER_ROOTS
        );
        for path in &self.cacerts {
            let mut file = io::BufReader::new(
                File::open(path).map_err(|err| {
                    error!(
                        "Unit {}: failed to open cacert file '{}': {}",
                        unit_name, path.display(), err
                    );
                    Terminated
                })?
            );
            match config.root_store.add_pem_file(&mut file) {
                Ok((good, bad)) => {
                    info!(
                        "Unit {}: cacert file '{}': \
                        added {} and skipped {} certificates.",
                        unit_name, path.display(), good, bad
                    );
                }
                Err(_) => {
                    error!(
                        "Unit {}: failed to read cacert file '{}.",
                        unit_name, path.display()
                    );
                    return Err(Terminated)
                }
            }
        }
        Ok(TlsConnector::from(Arc::new(config)))
    }

    async fn connect(
        state: Arc<TlsState>
    ) -> Result<TlsStream<MyTcpStream>, io::Error> {
        let stream = TcpStream::connect(&state.tls.remote).await?;
        state.connector.connect(state.domain.as_ref(), MyTcpStream { sock: stream }).await
    }
}


//------------ RtrClient -----------------------------------------------------

/// The transport-agnostic parts of a running RTR client.
#[derive(Debug)]
struct RtrClient<Connect> {
    /// The connect closure.
    connect: Connect,

    /// How long to wait before connecting again if the connection is closed.
    retry: u64,

    /// Our gate status.
    status: GateStatus,

    /// Our current serial.
    serial: Serial,
}

impl<Connect> RtrClient<Connect> {
    fn new(connect: Connect, retry: u64) -> Self {
        RtrClient {
            connect,
            retry,
            status: Default::default(),
            serial: Serial::default(),
        }
    }
}

impl<Connect, ConnectFut, Socket> RtrClient<Connect>
where
    Connect: FnMut() -> ConnectFut,
    ConnectFut: Future<Output = Result<Socket, io::Error>>,
    Socket: AsyncRead + AsyncWrite + Unpin,
{
    async fn run(
        mut component: Component,
        mut gate: Gate,
        retry: u64,
        connect: Connect,
    ) -> Result<(), Terminated> {
        let mut target = Target::new(component.name().clone());
        let metrics = Arc::new(RtrMetrics::new(&gate));
        component.register_metrics(metrics.clone());
        let mut this = Self::new(connect, retry);
        gate.update_status(UnitStatus::Stalled).await;
        loop {
            debug!("Unit {}: Connecting ...", target.name);
            let mut client = match this.connect(target, &mut gate).await {
                Ok(client) => {
                    gate.update_status(UnitStatus::Healthy).await;
                    client
                }
                Err(res) => {
                    debug!(
                        "Unit {}: Connection failed. Awaiting reconnect.",
                        res.name
                    );
                    gate.update_status(UnitStatus::Stalled).await;
                    this.retry_wait(&mut gate).await?;
                    target = res;
                    continue;
                }
            };

            loop {
                let update = match this.update(&mut client, &mut gate).await {
                    Ok(Ok(update)) => {
                        debug!(
                            "Unit {}: received update.", client.target().name
                        );
                        update
                    }
                    Ok(Err(_)) => {
                        debug!(
                            "Unit {}: RTR client disconnected.",
                            client.target().name
                        );
                        break;
                    }
                    Err(_) => {
                        debug!(
                            "Unit {}: RTR client terminated.",
                            client.target().name
                        );
                        return Err(Terminated)
                    }
                };
                if let Some(update) = update {
                    this.serial = update.serial();
                    client.target_mut().current = update.set().clone();
                    gate.update_data(update).await;
                }
            }

            target = client.into_target();
            gate.update_status(UnitStatus::Stalled).await;
            this.retry_wait(&mut gate).await?;
        }
    }

    async fn connect(
        &mut self, target: Target, gate: &mut Gate,
    ) -> Result<Client<Socket, Target>, Target> {
        let sock = {
            let connect = (self.connect)();
            pin_mut!(connect);

            loop {
                let process = gate.process();
                pin_mut!(process);
                match select(process, connect).await {
                    Either::Left((Err(_), _)) => {
                        return Err(target)
                    }
                    Either::Left((Ok(status), next_fut)) => {
                        self.status = status;
                        connect = next_fut;
                    }
                    Either::Right((res, _)) => break res
                }
            }
        };

        let sock = match sock {
            Ok(sock) => sock,
            Err(err) => {
                warn!("Unit {}: {}", target.name, err);
                return Err(target)
            }
        };

        let state = target.state;
        Ok(Client::new(sock, target, state))
    }

    async fn update(
        &mut self, client: &mut Client<Socket, Target>, gate: &mut Gate
    ) -> Result<Result<Option<payload::Update>, io::Error>, Terminated> {
        let next_serial = self.serial.add(1);
        let update_fut = async {
            let update = client.update().await?;
            if update.is_definitely_empty() {
                return Ok(None)
            }
            match update.into_update(next_serial) {
                Ok(res) => Ok(Some(res)),
                Err(err) => {
                    client.send_error(err).await?;
                    Err(io::Error::new(io::ErrorKind::Other, err))
                }
            }
        };
        pin_mut!(update_fut);

        loop {
            let process = gate.process();
            pin_mut!(process);
            match select(process, update_fut).await {
                Either::Left((Err(_), _)) => {
                    return Err(Terminated)
                }
                Either::Left((Ok(status), next_fut)) => {
                    self.status = status;
                    update_fut = next_fut;
                }
                Either::Right((res, _)) => {
                    return Ok(res)
                }
            }
        }
    }

    async fn retry_wait(
        &mut self, gate: &mut Gate
    ) -> Result<(), Terminated> {
        let end = Instant::now() + Duration::from_secs(self.retry);

        while end > Instant::now() {
            match timeout_at(end, gate.process()).await {
                Ok(Ok(status)) => {
                    self.status = status
                }
                Ok(Err(_)) => return Err(Terminated),
                Err(_) => return Ok(()),
            }
        }

        Ok(())
    }
}


//------------ Target --------------------------------------------------------

struct Target {
    current: payload::Set,

    state: Option<State>,

    name: Arc<str>,
}

impl Target {
    pub fn new(name: Arc<str>) -> Self {
        Target {
            current: Default::default(),
            state: None,
            name
        }
    }
}

impl PayloadTarget for Target {
    type Update = TargetUpdate;

    fn start(&mut self, reset: bool) -> Self::Update {
        debug!("Unit {}: starting update (reset={})", self.name, reset);
        if reset {
            TargetUpdate::Reset(payload::PackBuilder::empty())
        }
        else {
            TargetUpdate::Serial {
                set: self.current.clone(),
                diff: payload::DiffBuilder::empty(),
            }
        }
    }

    fn apply(
        &mut self,
        _update: Self::Update,
        _timing: Timing
    ) -> Result<(), PayloadError> {
        unreachable!()
    }
}


//------------ TargetUpdate --------------------------------------------------

enum TargetUpdate {
    Reset(payload::PackBuilder),
    Serial {
        set: payload::Set,
        diff: payload::DiffBuilder,
    }
}

impl TargetUpdate {
    fn is_definitely_empty(&self) -> bool {
        match *self {
            TargetUpdate::Reset(_) => false,
            TargetUpdate::Serial { ref diff, .. } => diff.is_empty()
        }
    }

    fn into_update(
        self, serial: Serial
    ) -> Result<payload::Update, PayloadError> {
        match self {
            TargetUpdate::Reset(pack) => {
                Ok(payload::Update::new(serial, pack.finalize().into(), None))
            }
            TargetUpdate::Serial { set, diff } => {
                let diff = diff.finalize();
                let set = diff.apply(&set)?;
                Ok(payload::Update::new(serial, set, Some(diff)))
            }
        }
    }
}

impl PayloadUpdate for TargetUpdate {
    fn push_update(
        &mut self,
        action: Action,
        payload: Payload
    ) -> Result<(), PayloadError> {
        match *self {
            TargetUpdate::Reset(ref mut pack) => {
                if action == Action::Withdraw {
                    Err(PayloadError::Corrupt)
                }
                else {
                    pack.insert(payload)
                }
            }
            TargetUpdate::Serial { ref mut diff, .. } => {
                diff.push(payload, action)
            }
        }
    }
}


//------------ RtrMetrics ----------------------------------------------------

#[derive(Debug, Default)]
struct RtrMetrics {
    gate: Arc<GateMetrics>,
}

impl RtrMetrics {
    fn new(gate: &Gate) -> Self {
        RtrMetrics {
            gate: gate.metrics(),
        }
    }
}

impl metrics::Source for RtrMetrics {
    fn append(&self, unit_name: &str, target: &mut metrics::Target)  {
        self.gate.append(unit_name, target);
    }
}


pin_project! {
    struct MyTcpStream { #[pin] sock: TcpStream }
}

impl AsyncRead for MyTcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>
    ) -> Poll<Result<(), io::Error>> {
        self.as_mut().project().sock.poll_read(cx, buf)
    }
}

impl AsyncWrite for MyTcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8]
    ) -> Poll<Result<usize, io::Error>> {
        self.as_mut().project().sock.poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>
    ) -> Poll<Result<(), io::Error>> {
        self.as_mut().project().sock.poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>
    ) -> Poll<Result<(), io::Error>> {
        self.as_mut().project().sock.poll_shutdown(cx)
    }
}

