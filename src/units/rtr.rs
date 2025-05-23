//! RTR client units.
//!
//! There are two units in this module that act as an RTR client but use
//! different transport protocols: [`Tcp`] uses plain, unencrypted TCP while
//! [`Tls`] uses TLS.

use std::io;
use std::fs::File;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64};
use std::task::{Context, Poll};
use std::time::Duration;
use chrono::{TimeZone, Utc};
use daemonbase::config::ConfigPath;
use futures_util::pin_mut;
use futures_util::future::{select, Either};
use log::{debug, error, warn};
use pin_project_lite::pin_project;
use rpki::rtr::client::{Client, PayloadError, PayloadTarget, PayloadUpdate};
use rpki::rtr::payload::{Action, Payload, Timing};
use rpki::rtr::state::State;
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::{timeout_at, Instant};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::rustls::pki_types::ServerName;
use crate::metrics;
use crate::comms::{Gate, GateMetrics, GateStatus, Terminated, UnitUpdate};
use crate::manager::Component;
use crate::metrics::{Metric, MetricType, MetricUnit};
use crate::payload;

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
    /// The default re-connect timeout in seconds.
    fn default_retry() -> u64 {
        60
    }

    /// Runs the unit.
    ///
    /// This method will only ever return if the RTR client encounters a fatal
    /// error.
    pub async fn run(
        self, component: Component, gate: Gate
    ) -> Result<(), Terminated> {
        let metrics = Arc::new(RtrMetrics::new(&gate));
        RtrClient::run(
            component, gate, self.retry, metrics.clone(),
            || async {
                Ok(RtrTcpStream {
                    sock: TcpStream::connect(&self.remote).await?,
                    metrics: metrics.clone()
                })
            }
        ).await
    }
}


//------------ Tls -----------------------------------------------------------

/// An RTR client using a TLS encrypted TCP socket.
#[derive(Debug, Deserialize)]
pub struct Tls {
    /// The remote address to connect to.
    remote: String,

    /// How long to wait before connecting again if the connection is closed.
    #[serde(default = "Tcp::default_retry")]
    retry: u64,

    /// Paths to root certficates.
    ///
    /// The files should contain one or more PEM-encoded certificates.
    #[serde(default)]
    cacerts: Vec<ConfigPath>,
}

/// Run-time information of the TLS unit.
struct TlsState {
    /// The unit configuration.
    tls: Tls,

    /// The name of the server.
    domain: ServerName<'static>,

    /// The TLS configuration for connecting to the server.
    connector: TlsConnector,

    /// The unit’s metrics.
    metrics: Arc<RtrMetrics>,
}

impl Tls {
    /// Runs the unit.
    ///
    /// This method will only ever return if the RTR client encounters a fatal
    /// error.
    pub async fn run(
        self, component: Component, gate: Gate
    ) -> Result<(), Terminated> {
        let domain = self.get_domain_name(component.name())?;
        let connector = self.build_connector(component.name())?;
        let retry = self.retry;
        let metrics = Arc::new(RtrMetrics::new(&gate));
        let state = Arc::new(TlsState {
            tls: self, domain, connector, metrics: metrics.clone(), 
        });
        RtrClient::run(
            component, gate, retry, metrics,
            move || {
                Self::connect(state.clone())
            }
        ).await
    }

    /// Converts the server address into the name for certificate validation.
    fn get_domain_name(
        &self, unit_name: &str
    ) -> Result<ServerName<'static>, Terminated> {
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
        ServerName::try_from(host).map(|res| res.to_owned()).map_err(|err| {
            error!(
                "Unit {}: Invalid remote name '{}': {}'",
                unit_name, host, err
            );
            Terminated
        })
    }

    /// Prepares the TLS configuration for connecting to the server.
    fn build_connector(
        &self, unit_name: &str
    ) -> Result<TlsConnector, Terminated> {
        let mut root_certs = RootCertStore {
            roots: Vec::from(webpki_roots::TLS_SERVER_ROOTS)
        };
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
            for cert in rustls_pemfile::certs(&mut file) {
                let cert = match cert {
                    Ok(cert) => cert,
                    Err(err) => {
                        error!(
                            "Unit {}: failed to read certificate file '{}': \
                             {}",
                            unit_name, path.display(), err
                        );
                        return Err(Terminated)
                    }
                };
                if let Err(err) = root_certs.add(cert) {
                    error!(
                        "Unit {}: failed to add TLS certificate \
                         from file '{}': {}",
                        unit_name, path.display(), err
                    );
                    return Err(Terminated)
                }
            }
        }

        Ok(TlsConnector::from(Arc::new(
            ClientConfig::builder()
                .with_root_certificates(root_certs)
                .with_no_client_auth()
        )))
    }

    /// Connects to the server.
    async fn connect(
        state: Arc<TlsState>
    ) -> Result<TlsStream<RtrTcpStream>, io::Error> {
        let stream = TcpStream::connect(&state.tls.remote).await?;
        state.connector.connect(
            state.domain.clone(),
            RtrTcpStream {
                sock: stream,
                metrics: state.metrics.clone(),
            }
        ).await
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

    /// The unit’s metrics.
    metrics: Arc<RtrMetrics>,
}

impl<Connect> RtrClient<Connect> {
    /// Creates a new client from the connect closure and retry timeout.
    fn new(connect: Connect, retry: u64, metrics: Arc<RtrMetrics>) -> Self {
        RtrClient {
            connect,
            retry,
            status: Default::default(),
            metrics,
        }
    }
}

impl<Connect, ConnectFut, Socket> RtrClient<Connect>
where
    Connect: FnMut() -> ConnectFut,
    ConnectFut: Future<Output = Result<Socket, io::Error>>,
    Socket: AsyncRead + AsyncWrite + Unpin,
{
    /// Runs the client.
    ///
    /// This method will only ever return if the RTR client encounters a fatal
    /// error.
    async fn run(
        mut component: Component,
        mut gate: Gate,
        retry: u64,
        metrics: Arc<RtrMetrics>,
        connect: Connect,
    ) -> Result<(), Terminated> {
        let mut target = Target::new(component.name().clone());
        component.register_metrics(metrics.clone());
        let mut this = Self::new(connect, retry, metrics);
        loop {
            debug!("Unit {}: Connecting ...", target.name);
            let mut client = match this.connect(target, &mut gate).await {
                Ok(client) => client,
                Err(res) => {
                    debug!(
                        "Unit {}: Connection failed. Awaiting reconnect.",
                        res.name
                    );
                    gate.update(UnitUpdate::Stalled).await;
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
                    Ok(Err(err)) => {
                        warn!(
                            "Unit {}: RTR client disconnected: {}",
                            client.target().name, err,
                        );
                        debug!(
                            "Unit {}: awaiting reconnect.",
                            client.target().name,
                        );
                        break;
                    }
                    Err(_) => {
                        debug!(
                            "Unit {}: RTR client terminated.",
                            client.target().name
                        );
                        gate.update(UnitUpdate::Gone).await;
                        return Err(Terminated)
                    }
                };
                if let Some(update) = update {
                    client.target_mut().current = update.set().clone();
                    gate.update(UnitUpdate::Payload(update)).await;
                }
            }

            target = client.into_target();
            gate.update(UnitUpdate::Stalled).await;
            this.retry_wait(&mut gate).await?;
        }
    }

    /// Connects to the server.
    ///
    /// Upon succes, returns an RTR client that wraps the provided target.
    /// Upon failure to connect, logs the reason and returns the target for
    /// later retry.
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
                warn!(
                    "Unit {}: failed to connect to server: {}",
                    target.name, err
                );
                return Err(target)
            }
        };

        let state = target.state;
        Ok(Client::new(sock, target, state))
    }

    /// Updates the data set from upstream.
    ///
    /// Waits until it is time to ask for an update or the server sends a
    /// notification and then asks for an update.
    ///
    /// This can fail fatally, in which case `Err(Terminated)` is returned and
    /// the client should shut down. This can also fail normally, i.e., the
    /// connections with the server fails or the server misbehaves. In this
    /// case, `Ok(Err(_))` is returned and the client should wait and try
    /// again.
    ///
    /// A successful update results in a (slightly passive-agressive)
    /// `Ok(Ok(_))`. If the client’s data set has changed, this change is
    /// returned, otherwise the fact that there are no changes is indicated
    /// via `None`.
    #[allow(clippy::needless_pass_by_ref_mut)] // false positive
    async fn update(
        &mut self, client: &mut Client<Socket, Target>, gate: &mut Gate
    ) -> Result<Result<Option<payload::Update>, io::Error>, Terminated> {
        let update_fut = async {
            let update = client.update().await?;
            let state = client.state();
            if update.is_definitely_empty() {
                return Ok((state, None))
            }
            match update.into_update() {
                Ok(res) => Ok((state, Some(res))),
                Err(err) => {
                    client.send_error(err).await?;
                    Err(io::Error::other(err))
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
                    let res = match res {
                        Ok((state, res)) => {
                            if let Some(state) = state {
                                self.metrics.session.store(
                                    state.session().into(),
                                    atomic::Ordering::Relaxed
                                );
                                self.metrics.serial.store(
                                    state.serial().into(),
                                    atomic::Ordering::Relaxed
                                );
                                self.metrics.updated.store(
                                    Utc::now().timestamp(),
                                    atomic::Ordering::Relaxed
                                );
                            }
                            Ok(res)
                        }
                        Err(err) => Err(err)
                    };
                    return Ok(res)
                }
            }
        }
    }

    /// Waits until we should retry connecting to the server.
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

/// The RPKI data target for the RTR client.
struct Target {
    /// The current payload set.
    current: payload::Set,

    /// The RTR client state.
    state: Option<State>,

    /// The component name.
    name: Arc<str>,
}

impl Target {
    /// Creates a new RTR target for the component with the given name.
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
        // This method is not used by the way we use the RTR client.
        unreachable!()
    }
}


//------------ TargetUpdate --------------------------------------------------

/// An update of the RPKI data set being assembled by the RTR client.
enum TargetUpdate {
    /// This is a reset query producing the complete data set.
    Reset(payload::PackBuilder),

    /// This is a serial query producing the difference to an earlier set.
    Serial {
        /// The current data set the differences are to be applied to.
        set: payload::Set,

        /// The differences as sent by the server.
        diff: payload::DiffBuilder,
    }
}

impl TargetUpdate {
    /// Returns whether there are definitely no changes in the update.
    fn is_definitely_empty(&self) -> bool {
        match *self {
            TargetUpdate::Reset(_) => false,
            TargetUpdate::Serial { ref diff, .. } => diff.is_empty()
        }
    }

    /// Converts the target update into a payload update.
    ///
    /// This will fail if the diff of a serial update doesn’t apply cleanly.
    fn into_update(self) -> Result<payload::Update, PayloadError> {
        match self {
            TargetUpdate::Reset(pack) => {
                Ok(payload::Update::new(pack.finalize().into()))
            }
            TargetUpdate::Serial { set, diff } => {
                let diff = diff.finalize();
                let set = diff.apply(&set)?;
                Ok(payload::Update::new(set))
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

/// The metrics for an RTR client.
#[derive(Debug, Default)]
struct RtrMetrics {
    /// The gate metrics.
    gate: Arc<GateMetrics>,

    /// The session ID of the last successful update.
    ///
    /// This is actually an `Option<u16>` with the value of `u32::MAX`
    /// serving as `None`.
    session: AtomicU32,

    /// The serial number of the last successful update.
    ///
    /// This is actually an option with the value of `u32::MAX` serving as
    /// `None`.
    serial: AtomicU32,

    /// The time the last successful update finished.
    ///
    /// This is an option of the unix timestamp. The value of `i64::MIN`
    /// serves as a `None`.
    updated: AtomicI64,

    /// The number of bytes read.
    bytes_read: AtomicU64,

    /// The number of bytes written.
    bytes_written: AtomicU64,
}

impl RtrMetrics {
    fn new(gate: &Gate) -> Self {
        RtrMetrics {
            gate: gate.metrics(),
            session: u32::MAX.into(),
            serial: u32::MAX.into(),
            updated: i64::MIN.into(),
            bytes_read: 0.into(),
            bytes_written: 0.into(),
        }
    }

    fn inc_bytes_read(&self, count: u64) {
        self.bytes_read.fetch_add(count, atomic::Ordering::Relaxed);
    }

    fn inc_bytes_written(&self, count: u64) {
        self.bytes_written.fetch_add(count, atomic::Ordering::Relaxed);
    }
}

impl RtrMetrics {
    const SESSION_METRIC: Metric = Metric::new(
        "session_id", "the session ID of the last successful update",
        MetricType::Text, MetricUnit::Info
    );
    const SERIAL_METRIC: Metric = Metric::new(
        "serial", "the serial number of the last successful update",
        MetricType::Counter, MetricUnit::Total
    );
    const UPDATED_AGO_METRIC: Metric = Metric::new(
        "since_last_rtr_update",
        "the number of seconds since last successful update",
        MetricType::Counter, MetricUnit::Total
    );
    const UPDATED_METRIC: Metric = Metric::new(
        "rtr_updated", "the time of the last successful update",
        MetricType::Text, MetricUnit::Info
    );
    const BYTES_READ_METRIC: Metric = Metric::new(
        "bytes_read", "the number of bytes read",
        MetricType::Counter, MetricUnit::Total,
    );
    const BYTES_WRITTEN_METRIC: Metric = Metric::new(
        "bytes_written", "the number of bytes written",
        MetricType::Counter, MetricUnit::Total,
    );

    const ISO_DATE: &'static [chrono::format::Item<'static>] = &[
        chrono::format::Item::Numeric(
            chrono::format::Numeric::Year, chrono::format::Pad::Zero
        ),
        chrono::format::Item::Literal("-"),
        chrono::format::Item::Numeric(
            chrono::format::Numeric::Month, chrono::format::Pad::Zero
        ),
        chrono::format::Item::Literal("-"),
        chrono::format::Item::Numeric(
            chrono::format::Numeric::Day, chrono::format::Pad::Zero
        ),
        chrono::format::Item::Literal("T"),
        chrono::format::Item::Numeric(
            chrono::format::Numeric::Hour, chrono::format::Pad::Zero
        ),
        chrono::format::Item::Literal(":"),
        chrono::format::Item::Numeric(
            chrono::format::Numeric::Minute, chrono::format::Pad::Zero
        ),
        chrono::format::Item::Literal(":"),
        chrono::format::Item::Numeric(
            chrono::format::Numeric::Second, chrono::format::Pad::Zero
        ),
        chrono::format::Item::Literal("Z"),
    ];
}

impl metrics::Source for RtrMetrics {
    fn append(&self, unit_name: &str, target: &mut metrics::Target)  {
        self.gate.append(unit_name, target);

        let session = self.session.load(atomic::Ordering::Relaxed);
        if session != u32::MAX {
            target.append_simple(
                &Self::SESSION_METRIC, Some(unit_name), session
            );
        }

        let serial = self.serial.load(atomic::Ordering::Relaxed);
        if serial != u32::MAX {
            target.append_simple(
                &Self::SERIAL_METRIC, Some(unit_name), serial
            )
        }

        let updated = self.updated.load(atomic::Ordering::Relaxed);
        if updated != i64::MIN {
            if let Some(updated) = Utc.timestamp_opt(updated, 0).single() {
                let ago = Utc::now().signed_duration_since(updated);
                target.append_simple(
                    &Self::UPDATED_AGO_METRIC, Some(unit_name),
                    ago.num_seconds()
                );
                target.append_simple(
                    &Self::UPDATED_METRIC, Some(unit_name),
                    updated.format_with_items(Self::ISO_DATE.iter())
                );
            }
        }

        target.append_simple(
            &Self::BYTES_READ_METRIC, Some(unit_name),
            self.bytes_read.load(atomic::Ordering::Relaxed)
        );
        target.append_simple(
            &Self::BYTES_WRITTEN_METRIC, Some(unit_name),
            self.bytes_written.load(atomic::Ordering::Relaxed)
        );
    }
}


//------------ RtrTcpStream --------------------------------------------------

pin_project! {
    /// A wrapper around a TCP socket producing metrics.
    struct RtrTcpStream {
        #[pin] sock: TcpStream,

        metrics: Arc<RtrMetrics>,
    }
}

impl AsyncRead for RtrTcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>
    ) -> Poll<Result<(), io::Error>> {
        let len = buf.filled().len();
        let res = self.as_mut().project().sock.poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = res {
            self.metrics.inc_bytes_read(
                (buf.filled().len().saturating_sub(len)) as u64
            )    
        }
        res
    }
}

impl AsyncWrite for RtrTcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8]
    ) -> Poll<Result<usize, io::Error>> {
        let res = self.as_mut().project().sock.poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = res {
            self.metrics.inc_bytes_written(n as u64)
        }
        res
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

