/// RTR servers as a target.

use std::{cmp, io};
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{
    AtomicI64, AtomicU32, AtomicU64, AtomicUsize,
};
use std::sync::atomic::Ordering::Relaxed;
use std::net::{IpAddr, SocketAddr};
use std::net::TcpListener as StdTcpListener;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use arc_swap::ArcSwap;
use chrono::{DateTime, TimeZone, Utc};
use daemonbase::config::ConfigPath;
use daemonbase::error::ExitError;
use futures_util::{Stream, pin_mut};
use log::{debug, error};
use serde::Deserialize;
use rpki::rtr::payload::Timing;
use rpki::rtr::server::{NotifySender, Server, Socket, PayloadSource};
use rpki::rtr::state::{Serial, State};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use crate::{metrics, payload};
use crate::comms::{Link, UnitUpdate};
use crate::manager::Component;
use crate::metrics::{Metric, MetricType, MetricUnit};
use crate::utils::tls;
use crate::utils::tls::MaybeTlsTcpStream;


//------------ Tcp -----------------------------------------------------------

/// An RTR server atop unencrypted, plain TCP.
#[derive(Debug, Deserialize)]
pub struct Tcp {
    /// The socket addresses to listen on.
    listen: Vec<SocketAddr>,

    /// The unit whose data set we should serve.
    unit: Link,

    /// The maximum number of deltas we should keep.
    #[serde(default = "Tcp::default_history_size")]
    #[serde(rename = "history-size")]
    history_size: usize,

    /// The RTR refresh interval.
    refresh: Option<u32>,

    /// The RTR retry interval.
    retry: Option<u32>,

    /// The RTR expire interval.
    expire: Option<u32>,

    /// Keep per-client metrics?
    #[serde(default)]
    #[serde(rename = "client-metrics")]
    client_metrics: bool,
}

impl Tcp {
    /// The default for the `history_size` value.
    const fn default_history_size() -> usize {
        10
    }

    /// Runs the target.
    pub async fn run(
        self, mut component: Component
    ) -> Result<(), ExitError> {
        let notify = NotifySender::new();
        let target = Source::new(self.history_size, self.timing());
        let metrics = Arc::new(ListenerMetrics::new(self.client_metrics));
        component.register_metrics(metrics.clone());

        for &addr in &self.listen {
            RtrListener::spawn(
                addr, None, None,
                target.clone(), notify.clone(), metrics.clone()
            )?;
        }

        self.run_loop(component, target, notify).await
    }

    /// Runs the target’s main loop.
    async fn run_loop(
        mut self,
        component: Component,
        target: Source,
        mut notify: NotifySender,
    ) -> Result<(), ExitError> {
        loop {
            let update = self.unit.query().await;
            if let UnitUpdate::Payload(ref payload) = update {
                debug!(
                    "Target {}: Got update ({} entries)",
                    component.name(), payload.set().len()
                );
            }
            if target.update(update) {
                notify.notify()
            }
        }
    }

    fn timing(&self) -> Timing {
        let mut res = Timing::default();
        if let Some(refresh) = self.refresh {
            res.refresh = refresh;
        }
        if let Some(retry) = self.retry {
            res.retry = retry;
        }
        if let Some(expire) = self.expire {
            res.expire = expire;
        }
        res
    }
}


//------------ Tls -----------------------------------------------------------

/// An RTR server atop TLS.
#[derive(Debug, Deserialize)]
pub struct Tls {
    /// The configuration values shared with [`Tcp`].
    #[serde(flatten)]
    tcp: Tcp,

    /// The path to the server certificate to present to clients.
    certificate: ConfigPath,

    /// The path to the private key to use for encryption.
    key: ConfigPath,
}

impl Tls {
    /// Runs the target.
    pub async fn run(
        self, mut component: Component
    ) -> Result<(), ExitError> {
        let acceptor = TlsAcceptor::from(Arc::new(
            tls::create_server_config(
                component.name(), &self.certificate, &self.key
            )?
        ));
        let notify = NotifySender::new();
        let target = Source::new(self.tcp.history_size, self.tcp.timing());
        let metrics = Arc::new(ListenerMetrics::new(self.tcp.client_metrics));
        component.register_metrics(metrics.clone());

        for &addr in &self.tcp.listen {
            RtrListener::spawn(
                addr, Some(acceptor.clone()), None,
                target.clone(), notify.clone(), metrics.clone(),
            )?;
        }

        self.tcp.run_loop(component, target, notify).await
    }
}


//============ Data ==========================================================

//------------ Source --------------------------------------------------------

/// The data source for the RTR client.
#[derive(Clone)]
struct Source {
    /// The current data set.
    data: Arc<ArcSwap<SourceData>>,

    /// The maximum nummber of diffs to keep.
    history_size: usize,
    timing: Timing,
}

impl Source {
    /// Creates a new source using the given history size and timing.
    fn new(history_size: usize, timing: Timing) -> Self {
        Source {
            data: Default::default(),
            history_size,
            timing,
        }
    }

    /// Updates the source from the provided unit update.
    ///
    /// Returns whether there is a new data set and clients need notifying.
    fn update(&self, update: UnitUpdate) -> bool {
        let payload = match update {
            UnitUpdate::Payload(payload) => payload,
            _ => return false,
        };

        let data = self.data.load();
        let new_data = match data.current.as_ref() {
            None => {
                SourceData {
                    state: data.state,
                    current: Some(payload.set().clone()),
                    diffs: Vec::new(),
                    timing: self.timing,
                }
            }
            Some(current) => {
                let diff = payload.set().diff_from(current);
                if diff.is_empty() {
                    // If there is no change in data, don’t update.
                    return false
                }
                let mut diffs = Vec::with_capacity(
                    cmp::min(data.diffs.len() + 1, self.history_size)
                );
                diffs.push((data.state.serial(), diff.clone()));
                for (serial, old_diff) in &data.diffs {
                    if diffs.len() >= self.history_size {
                        break
                    }
                    diffs.push((
                        *serial,
                        old_diff.extend(&diff).unwrap()
                    ))
                }
                let mut state = data.state;
                state.inc();
                SourceData {
                    state,
                    current: Some(payload.set().clone()),
                    diffs,
                    timing: self.timing,
                }
            }
        };

        self.data.store(new_data.into());
        true
    }
}

impl PayloadSource for Source {
    type Set = payload::OwnedSetIter;
    type Diff = payload::OwnedDiffIter;

    fn ready(&self) -> bool {
        self.data.load().current.is_some()
    }

    fn notify(&self) -> State {
        self.data.load().state
    }

    fn full(&self) -> (State, Self::Set) {
        let this = self.data.load();
        match this.current.as_ref() {
            Some(current) => (this.state, current.owned_iter()),
            None => (this.state, payload::Set::default().owned_iter()),
        }
    }

    fn diff(&self, state: State) -> Option<(State, Self::Diff)> {
        let this = self.data.load();
        if this.current.is_none() || state.session() != this.state.session() {
            return None
        }

        this.get_diff(state.serial()).map(|diff| {
            (this.state, diff)
        })
    }

    fn timing(&self) -> Timing {
        self.data.load().timing
    }
}


//------------ SourceData ----------------------------------------------------

/// The RTR data set.
#[derive(Clone, Default)]
struct SourceData {
    /// The current RTR state of the target.
    state: State,

    /// The current set of RTR data.
    current: Option<payload::Set>,

    /// The diffs we currently keep.
    ///
    /// The diff with the largest serial is first.
    diffs: Vec<(Serial, payload::Diff)>,

    /// The timing paramters for this source.
    timing: Timing,
}

impl SourceData {
    /// Returns the diff for the given serial if available.
    fn get_diff(&self, serial: Serial) -> Option<payload::OwnedDiffIter> {
        if serial == self.state.serial() {
            Some(payload::Diff::default().into_owned_iter())
        }
        else {
            self.diffs.iter().find_map(|item| {
                if item.0 == serial {
                    Some(item.1.owned_iter())
                }
                else {
                    None
                }
            })
        }
    }
}


//============ Sockets =======================================================

//------------ RtrListener --------------------------------------------------

/// A wrapper around an TCP listener that produces RTR streams.
struct RtrListener {
    tcp: TcpListener,
    tls: Option<TlsAcceptor>,
    keepalive: Option<Duration>,
    server_metrics: Arc<ListenerMetrics>,
}

impl RtrListener {
    fn spawn(
        addr: SocketAddr,
        tls: Option<TlsAcceptor>,
        keepalive: Option<Duration>,
        target: Source,
        notify: NotifySender,
        server_metrics: Arc<ListenerMetrics>,
    ) -> Result<(), ExitError> {
        let listener = match StdTcpListener::bind(addr) {
            Ok(listener) => listener,
            Err(err) => {
                error!("Can’t bind to {}: {}", addr, err);
                return Err(ExitError::default())
            }
        };
        if let Err(err) = listener.set_nonblocking(true) {
            error!(
                "Fatal: failed to set listener {} to non-blocking: {}.",
                addr, err
            );
            return Err(ExitError::default());
        }
        let listener = match TcpListener::from_std(listener) {
            Ok(tcp) => Self { tcp, tls, keepalive, server_metrics },
            Err(err) => {
                error!("Fatal error listening on {}: {}", addr, err);
                return Err(ExitError::default())
            }
        };
        tokio::spawn(async move {
            let server = Server::new(listener, notify, target);
            if server.run().await.is_err() {
                error!("Fatal error in RTR server on {}.", addr);
            }
        });
        Ok(())
    }
}

impl Stream for RtrListener {
    type Item = Result<RtrStream, io::Error>;

    fn poll_next(
        self: Pin<&mut Self>,
        ctx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match self.tcp.poll_accept(ctx) {
            Poll::Ready(Ok((sock, addr))) => {
                match RtrStream::new(
                    sock, addr,
                    self.tls.as_ref(),
                    self.keepalive,
                    &self.server_metrics
                ) {
                    Ok(stream) => Poll::Ready(Some(Ok(stream))),
                    Err(_) => Poll::Pending,
                }
            }
            Poll::Ready(Err(err)) => Poll::Ready(Some(Err(err))),
            Poll::Pending => Poll::Pending,
        }
    }
}


//------------ RtrStream ----------------------------------------------------

/// A wrapper around a stream socket that takes care of updating metrics.
struct RtrStream {
    sock: MaybeTlsTcpStream,
    metrics: ClientMetrics,
}

impl RtrStream {
    #[allow(clippy::redundant_async_block)] // False positive
    fn new(
        sock: TcpStream,
        addr: SocketAddr,
        tls: Option<&TlsAcceptor>,
        keepalive: Option<Duration>,
        server_metrics: &ListenerMetrics,
    ) -> Result<Self, io::Error> {
        if let Some(duration) = keepalive {
            Self::set_keepalive(&sock, duration)?
        }
        let metrics = server_metrics.get_client(addr.ip());
        metrics.update(|metrics| metrics.inc_open());
        Ok(RtrStream {
            sock: MaybeTlsTcpStream::new(sock, tls),
            metrics
        })
    }

    #[cfg(unix)]
    fn set_keepalive(
        sock: &TcpStream, duration: Duration
    ) -> Result<(), io::Error>{
        use nix::sys::socket::{setsockopt, sockopt};

        (|fd, duration: Duration| {
            setsockopt(fd, sockopt::KeepAlive, &true)?;

            // The attributes are copied from the definitions in
            // nix::sys::socket::sockopt. Let’s hope they never change.

            #[cfg(any(target_os = "ios", target_os = "macos"))]
            setsockopt(
                fd, sockopt::TcpKeepAlive,
                &u32::try_from(duration.as_secs()).unwrap_or(u32::MAX)
            )?;

            #[cfg(any(
                target_os = "android",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "linux",
                target_os = "nacl"
            ))]
            setsockopt(
                fd, sockopt::TcpKeepIdle,
                &u32::try_from(duration.as_secs()).unwrap_or(u32::MAX)
            )?;

            #[cfg(not(target_os = "openbsd"))]
            setsockopt(
                fd, sockopt::TcpKeepInterval,
                &u32::try_from(duration.as_secs()).unwrap_or(u32::MAX)
            )?;

            Ok(())
        })(sock, duration).map_err(|err: nix::errno::Errno| {
            io::Error::new(io::ErrorKind::Other, err)
        })
    }

    #[cfg(not(unix))]
    fn set_keepalive(
        _sock: &TcpStream, _duration: Duration
    ) -> Result<(), io::Error>{
        Ok(())
    }
}

impl Socket for RtrStream {
    fn update(&self, state: State, reset: bool) {
        self.metrics.update(|metrics| {
            metrics.update_now(state.serial(), reset)
        });
    }
}

impl AsyncRead for RtrStream {
    fn poll_read(
        mut self: Pin<&mut Self>, cx: &mut Context, buf: &mut ReadBuf
    ) -> Poll<Result<(), io::Error>> {
        let len = buf.filled().len();
        let sock = &mut self.sock;
        pin_mut!(sock);
        let res = sock.poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = res {
            let len = buf.filled().len().saturating_sub(len) as u64;
            self.metrics.update(|metrics| metrics.inc_bytes_read(len));
        }
        res
    }
}

impl AsyncWrite for RtrStream {
    fn poll_write(
        mut self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]
    ) -> Poll<Result<usize, io::Error>> {
        let sock = &mut self.sock;
        pin_mut!(sock);
        let res = sock.poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = res {
            self.metrics.update(|metrics| metrics.inc_bytes_written(n as u64))
        }
        res
    }

    fn poll_flush(
        mut self: Pin<&mut Self>, cx: &mut Context
    ) -> Poll<Result<(), io::Error>> {
        let sock = &mut self.sock;
        pin_mut!(sock);
        sock.poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>, cx: &mut Context
    ) -> Poll<Result<(), io::Error>> {
        let sock = &mut self.sock;
        pin_mut!(sock);
        sock.poll_shutdown(cx)
    }
}

impl Drop for RtrStream {
    fn drop(&mut self) {
        self.metrics.update(|metrics| metrics.dec_open())
    }
}


//============ Metrics =======================================================

//------------ ListenerMetrics -----------------------------------------------

/// The metrics held by a listener.
struct ListenerMetrics {
    /// The global metrics over all connections.
    global: Arc<MetricsData>,

    /// The per-client address metrics.
    ///
    /// If this is `None`, per-client metrics are disabled.
    client: Option<PerAddrMetrics>,
}

impl ListenerMetrics {
    fn new(client_metrics: bool) -> Self {
        Self {
            global: Default::default(),
            client: client_metrics.then(Default::default),
        }
    }

    fn get_client(&self, addr: IpAddr) -> ClientMetrics {
        ClientMetrics {
            global: self.global.clone(),
            client: self.client.as_ref().map(|client| client.get(addr)),
        }
    }
}

impl metrics::Source for ListenerMetrics {
    fn append(&self, unit_name: &str, target: &mut metrics::Target)  {
        if let Some(client) = self.client.as_ref() {
            let client = client.all().iter().map(|(k, v)| {
                (k.to_string(), v.clone())
            }).collect::<Vec<_>>();

            target.append(
                &Self::CLIENT_OPEN_METRIC, Some(unit_name),
                |records| {
                    for (addr, metric) in &client {
                        records.label_value(&[("addr", addr)], metric.open());
                    }
                }
            );
            target.append(
                &Self::SERIAL_METRIC, Some(unit_name),
                |records| {
                    for (addr, metric) in &client {
                        match metric.serial() {
                            Some(serial) => {
                                records.label_value(
                                    &[("addr", addr)], serial
                                );
                            }
                            None => {
                                records.label_value(
                                    &[("addr", addr)], "-1"
                                );
                            }
                        }
                    }
                }
            );
            target.append(
                &Self::UPDATED_METRIC, Some(unit_name),
                |records| {
                    for (addr, metric) in &client {
                        match metric.updated() {
                            Some(time) => {
                                let duration = Utc::now() - time;
                                records.label_value(
                                    &[("addr", addr)],
                                    format_args!(
                                        "{}.{:03}",
                                        duration.num_seconds(),
                                        duration.num_milliseconds() % 1000,
                                    )
                                );
                            }
                            None => {
                                records.label_value(
                                    &[("addr", addr)], "-1"
                                );
                            }
                        }
                    }
                }
            );
            target.append(
                &Self::LAST_RESET_METRIC, Some(unit_name),
                |records| {
                    for (addr, metric) in &client {
                        match metric.last_reset() {
                            Some(time) => {
                                let duration = Utc::now() - time;
                                records.label_value(
                                    &[("addr", addr)],
                                    format_args!(
                                        "{}.{:03}",
                                        duration.num_seconds(),
                                        duration.num_milliseconds() % 1000,
                                    )
                                );
                            }
                            None => {
                                records.label_value(
                                    &[("addr", addr)], "-1"
                                );
                            }
                        }
                    }
                }
            );
            target.append(
                &Self::RESET_QUERIES_METRIC, Some(unit_name),
                |records| {
                    for (addr, metric) in &client {
                        records.label_value(
                            &[("addr", addr)],
                            metric.reset_queries()
                        );
                    }
                }
            );
            target.append(
                &Self::SERIAL_QUERIES_METRIC, Some(unit_name),
                |records| {
                    for (addr, metric) in &client {
                        records.label_value(
                            &[("addr", addr)],
                            metric.serial_queries()
                        );
                    }
                }
            );
            target.append(
                &Self::CLIENT_READ_METRIC, Some(unit_name),
                |records| {
                    for (addr, metric) in &client {
                        records.label_value(
                            &[("addr", addr)],
                            metric.bytes_read()
                        );
                    }
                }
            );
            target.append(
                &Self::CLIENT_WRITE_METRIC, Some(unit_name),
                |records| {
                    for (addr, metric) in &client {
                        records.label_value(
                            &[("addr", addr)],
                            metric.bytes_written()
                        );
                    }
                }
            );
        }

        target.append_simple(
            &Self::OPEN_METRIC, Some(unit_name), self.global.open()
        );
        target.append_simple(
            &Self::READ_METRIC, Some(unit_name), self.global.bytes_read()
        );
        target.append_simple(
            &Self::WRITE_METRIC, Some(unit_name), self.global.bytes_written()
        );
    }
}

impl ListenerMetrics {
    const CLIENT_OPEN_METRIC: Metric = Metric::new(
        "rtr_client_connections",
        "number of open client connections by a client address",
        MetricType::Gauge, MetricUnit::Total
    );
    const SERIAL_METRIC: Metric = Metric::new(
        "rtr_client_serial", "last serial seen by a client address",
        MetricType::Gauge, MetricUnit::Total
    );
    const UPDATED_METRIC: Metric = Metric::new(
        "rtr_client_last_update",
        "seconds since last update by a client address",
        MetricType::Gauge, MetricUnit::Second
    );
    const LAST_RESET_METRIC: Metric = Metric::new(
        "rtr_client_last_reset",
        "seconds since last cache reset by a client address",
        MetricType::Gauge, MetricUnit::Second
    );
    const RESET_QUERIES_METRIC: Metric = Metric::new(
        "rtr_client_reset_queries",
        "number of reset queries by a client address",
        MetricType::Counter, MetricUnit::Total
    );
    const SERIAL_QUERIES_METRIC: Metric = Metric::new(
        "rtr_client_serial_queries",
        "number of serial queries by a client address",
        MetricType::Counter, MetricUnit::Total
    );
    const CLIENT_READ_METRIC: Metric = Metric::new(
        "rtr_client_read",
        "number of bytes read from a client address",
        MetricType::Counter, MetricUnit::Byte
    );
    const CLIENT_WRITE_METRIC: Metric = Metric::new(
        "rtr_client_write",
        "number of bytes written to a client address",
        MetricType::Counter, MetricUnit::Byte
    );
    const OPEN_METRIC: Metric = Metric::new(
        "rtr_connections",
        "number of currently open RTR client connections",
        MetricType::Gauge, MetricUnit::Total
    );
    const READ_METRIC: Metric = Metric::new(
        "rtr_read",
        "number of bytes read by an RTR target",
        MetricType::Counter, MetricUnit::Byte
    );
    const WRITE_METRIC: Metric = Metric::new(
        "rtr_write",
        "number of bytes written by an RTR target",
        MetricType::Counter, MetricUnit::Byte
    );
}


//------------ PerAddrMetrics ------------------------------------------------

/// A map of metrics per client address.
#[derive(Debug, Default)]
struct PerAddrMetrics {
    addrs: ArcSwap<Vec<(IpAddr, Arc<MetricsData>)>>,
    write: Mutex<()>,
}

impl PerAddrMetrics {
    fn get(&self, addr: IpAddr) -> Arc<MetricsData> {
        // See if we have that address already.
        let addrs = self.addrs.load();
        if let Ok(idx) = addrs.binary_search_by(|x| x.0.cmp(&addr)) {
            return addrs[idx].1.clone()
        }

        // We don’t. Create a new slice with the address included.
        let _write = self.write.lock().expect("poisoned lock");

        // Re-load self.addrs, it may have changed since.
        let addrs = self.addrs.load();
        let idx = match addrs.binary_search_by(|x| x.0.cmp(&addr)) {
            Ok(idx) => return addrs[idx].1.clone(),
            Err(idx) => idx,
        };

        // Make a new self.addrs, by placing the new item in the right spot,
        // it’ll be automatically sorted.
        let mut new_addrs = Vec::with_capacity(addrs.len() + 1);
        new_addrs.extend_from_slice(&addrs[..idx]);
        new_addrs.push((addr, Default::default()));
        new_addrs.extend_from_slice(&addrs[idx..]);
        let res = new_addrs[idx].1.clone();
        self.addrs.store(new_addrs.into());
        res
    }

    fn all(&self) -> impl Deref<Target = Arc<Vec<(IpAddr, Arc<MetricsData>)>>> {
        self.addrs.load()
    }
}


//------------ ClientMetrics -------------------------------------------------

/// The metrics held by a connection.
#[derive(Debug)]
struct ClientMetrics {
    global: Arc<MetricsData>,
    client: Option<Arc<MetricsData>>,
}

impl ClientMetrics {
    fn update(&self, op: impl Fn(&MetricsData)) {
        op(&self.global);
        if let Some(client) = self.client.as_ref() {
            op(client)
        }
    }
}


//------------ MetricsData ---------------------------------------------------

#[derive(Debug)]
pub struct MetricsData {
    /// The number of currently open connections.
    open: AtomicUsize,

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

    /// The time the last successful cache reset finished.
    ///
    /// This is an option of the unix timestamp. The value of `i64::MIN`
    /// serves as a `None`.
    last_reset: AtomicI64,

    /// The number of successful reset queries.
    reset_queries: AtomicU32,

    /// The number of successful serial queries.
    serial_queries: AtomicU32,

    /// The number of bytes read.
    bytes_read: AtomicU64,

    /// The number of bytes written.
    bytes_written: AtomicU64,
}

impl Default for MetricsData {
    fn default() -> Self {
        Self {
            open: AtomicUsize::new(0),
            serial: AtomicU32::new(u32::MAX),
            updated: AtomicI64::new(i64::MIN),
            last_reset: AtomicI64::new(i64::MIN),
            reset_queries: AtomicU32::new(0),
            serial_queries: AtomicU32::new(0),
            bytes_read: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
        }
    }
}

impl MetricsData {
    fn open(&self) -> usize {
        self.open.load(Relaxed)
    }

    /// Increases the count of open connections.
    fn inc_open(&self) {
        self.open.fetch_add(1, Relaxed);
    }

    /// Decreases the count of open connections.
    fn dec_open(&self) {
        self.open.fetch_sub(1, Relaxed);
    }

    fn serial(&self) -> Option<u32> {
        match self.serial.load(Relaxed) {
            u32::MAX => None,
            other => Some(other),
        }
    }

    /// A successful update with the given serial number has finished now.
    ///
    /// Updates the serial number and update time accordingly.
    fn update_now(&self, serial: Serial, reset: bool) {
        self.serial.store(serial.into(), Relaxed);
        self.updated.store(Utc::now().timestamp(), Relaxed);
        if reset {
            self.last_reset.store(Utc::now().timestamp(), Relaxed);
            self.reset_queries.fetch_add(1, Relaxed);
        }
        else {
            self.serial_queries.fetch_add(1, Relaxed);
        }
    }

    /// Returns the time of the last successful update.
    ///
    /// Returns `None` if there never was a successful update.
    fn updated(&self) -> Option<DateTime<Utc>> {
        match self.updated.load(Relaxed) {
            i64::MIN => None,
            other => Utc.timestamp_opt(other, 0).single()
        }
    }

    /// Returns the time of the last successful reset update.
    ///
    /// Returns `None` if there never was a successful update.
    fn last_reset(&self) -> Option<DateTime<Utc>> {
        match self.last_reset.load(Relaxed) {
            i64::MIN => None,
            other => Utc.timestamp_opt(other, 0).single()
        }
    }

    /// Returns the number of successful reset queries.
    fn reset_queries(&self) -> u32 {
        self.reset_queries.load(Relaxed)
    }

    /// Returns the number of successful serial queries.
    fn serial_queries(&self) -> u32 {
        self.serial_queries.load(Relaxed)
    }

    /// Returns the total number of bytes read from this client.
    fn bytes_read(&self) -> u64 {
        self.bytes_read.load(Relaxed)
    }

    /// Increases the number of bytes read from this client.
    fn inc_bytes_read(&self, count: u64) {
        self.bytes_read.fetch_add(count, Relaxed);
    }

    /// Returns the total number of bytes written to this client.
    fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Relaxed)
    }

    /// Increases the number of bytes written to this client.
    fn inc_bytes_written(&self, count: u64) {
        self.bytes_written.fetch_add(count, Relaxed);
    }
}

