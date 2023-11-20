/// RTR servers as a target.

use std::{cmp, io};
use std::fs::File;
use std::sync::Arc;
use std::net::SocketAddr;
use std::net::TcpListener as StdTcpListener;
use std::pin::Pin;
use std::task::{Context, Poll};
use arc_swap::ArcSwap;
use daemonbase::config::ConfigPath;
use daemonbase::error::ExitError;
use futures::{TryFuture, ready};
use log::{debug, error};
use pin_project_lite::pin_project;
use serde::Deserialize;
use rpki::rtr::payload::Timing;
use rpki::rtr::server::{NotifySender, Server, PayloadSource};
use rpki::rtr::state::{Serial, State};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{Accept, TlsAcceptor};
use tokio_rustls::rustls::{Certificate, PrivateKey, ServerConfig};
use tokio_rustls::server::TlsStream;
use tokio_stream::wrappers::TcpListenerStream;
use crate::payload;
use crate::comms::Link;
use crate::manager::Component;


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
}

impl Tcp {
    /// The default for the `history_size` value.
    const fn default_history_size() -> usize {
        10
    }

    /// Runs the target.
    pub async fn run(mut self, component: Component) -> Result<(), ExitError> {
        let mut notify = NotifySender::new();
        let target = Source::new(self.history_size);
        for &addr in &self.listen {
            self.spawn_listener(addr, target.clone(), notify.clone())?;
        }

        loop {
            if let Ok(update) = self.unit.query().await {
                debug!(
                    "Target {}: Got update ({} entries)",
                    component.name(), update.set().len()
                );
                target.update(update);
                notify.notify()
            }
        }
    }

    /// Spawns a single listener onto the current runtime.
    fn spawn_listener(
        &self, addr: SocketAddr, target: Source, notify: NotifySender,
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
            Ok(listener) => listener,
            Err(err) => {
                error!("Fatal error listening on {}: {}", addr, err);
                return Err(ExitError::default())
            }
        };
        tokio::spawn(async move {
            let listener = TcpListenerStream::new(listener);
            let server = Server::new(listener, notify, target);
            if server.run().await.is_err() {
                error!("Fatal error listening on {}.", addr);
            }
        });
        Ok(())
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
    pub async fn run(mut self, component: Component) -> Result<(), ExitError> {
        let acceptor = TlsAcceptor::from(Arc::new(self.create_tls_config()?));
        let mut notify = NotifySender::new();
        let target = Source::new(self.tcp.history_size);
        for &addr in &self.tcp.listen {
            self.spawn_listener(
                addr, acceptor.clone(), target.clone(), notify.clone()
            )?;
        }

        loop {
            if let Ok(update) = self.tcp.unit.query().await {
                debug!(
                    "Target {}: Got update ({} entries)",
                    component.name(), update.set().len()
                );
                target.update(update);
                notify.notify()
            }
        }
    }

    /// Creates the TLS server config.
    fn create_tls_config(&self) -> Result<ServerConfig, ExitError> {
        let certs = rustls_pemfile::certs(
            &mut io::BufReader::new(
                File::open(&self.certificate).map_err(|err| {
                    error!(
                        "Failed to open TLS certificate file '{}': {}.",
                        self.certificate.display(), err
                    );
                    ExitError::default()
                })?
            )
        ).map_err(|err| {
            error!(
                "Failed to read TLS certificate file '{}': {}.",
                self.certificate.display(), err
            );
            ExitError::default()
        }).map(|mut certs| {
            certs.drain(..).map(Certificate).collect()
        })?;

        let key = rustls_pemfile::pkcs8_private_keys(
            &mut io::BufReader::new(
                File::open(&self.key).map_err(|err| {
                    error!(
                        "Failed to open TLS key file '{}': {}.",
                        self.key.display(), err
                    );
                    ExitError::default()
                })?
            )
        ).map_err(|err| {
            error!(
                "Failed to read TLS key file '{}': {}.",
                self.key.display(), err
            );
            ExitError::default()
        }).and_then(|mut certs| {
            if certs.is_empty() {
                error!(
                    "TLS key file '{}' does not contain any usable keys.",
                    self.key.display()
                );
                return Err(ExitError::default())
            }
            if certs.len() != 1 {
                error!(
                    "TLS key file '{}' contains multiple keys.",
                    self.key.display()
                );
                return Err(ExitError::default())
            }
            Ok(PrivateKey(certs.pop().unwrap()))
        })?;

        ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|err| {
                error!("Failed to create TLS server config: {}", err);
                ExitError::default()
            })
    }

    /// Spawns a single listener onto the current runtime.
    fn spawn_listener(
        &self, addr: SocketAddr, config: TlsAcceptor,
        target: Source, notify: NotifySender,
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
            Ok(listener) => listener,
            Err(err) => {
                error!("Fatal error listening on {}: {}", addr, err);
                return Err(ExitError::default())
            }
        };
        tokio::spawn(async move {
            use futures::StreamExt;

            let listener = TcpListenerStream::new(listener).map(|sock| {
                sock.map(|sock| TlsSocket::new(&config , sock))
            });
            let server = Server::new(listener, notify, target);
            if server.run().await.is_err() {
                error!("Fatal error listening on {}.", addr);
            }
        });
        Ok(())
    }

}


//------------ Source --------------------------------------------------------

#[derive(Clone)]
struct Source {
    data: Arc<ArcSwap<SourceData>>,
    history_size: usize,
}

impl Source {
    fn new(history_size: usize) -> Self {
        Source {
            data: Default::default(),
            history_size
        }
    }

    fn update(&self, update: payload::Update) {
        let data = self.data.load();

        let new_data = match data.current.as_ref() {
            None => {
                SourceData {
                    state: data.state,
                    unit_serial: update.serial(),
                    current: Some(update.set().clone()),
                    diffs: Vec::new(),
                    timing: Timing::default(),
                }
            }
            Some(current) => {
                let diff = match update.get_usable_diff(data.unit_serial) {
                    Some(diff) => diff.clone(),
                    None => update.set().diff_from(current),
                };
                if diff.is_empty() {
                    // If there is no change in data, don’t update.
                    return
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
                    unit_serial: update.serial(),
                    current: Some(update.set().clone()),
                    diffs,
                    timing: Timing::default(),
                }
            }
        };

        self.data.store(new_data.into());
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

#[derive(Clone, Default)]
struct SourceData {
    /// The current RTR state of the target.
    state: State,

    /// The current serial of the source unit.
    unit_serial: Serial,

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


//----------- TlsSocket ------------------------------------------------------

pin_project! {
    #[project = TlsSocketProj]
    enum TlsSocket {
        Accept { #[pin] fut: Accept<MyTcpStream> },
        Stream { #[pin] fut: TlsStream<MyTcpStream> },
        Empty,
    }
}

impl TlsSocket {
    fn new(acceptor: &TlsAcceptor, sock: TcpStream) -> Self {
        Self::Accept { fut: acceptor.accept(MyTcpStream { sock }) }
    }

    fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Pin<&mut Self>, io::Error>> {
        match self.as_mut().project() {
            TlsSocketProj::Accept { fut } => {
                match ready!(fut.try_poll(cx)) {
                    Ok(fut) => {
                        self.set(Self::Stream { fut });
                        Poll::Ready(Ok(self))
                    }
                    Err(err) => {
                        self.set(Self::Empty);
                        Poll::Ready(Err(err))
                    }
                }
            }
            TlsSocketProj::Stream { .. } => Poll::Ready(Ok(self)),
            TlsSocketProj::Empty => panic!("polling a concluded future")
        }
    }
}

impl rpki::rtr::server::Socket for TlsSocket { }

impl AsyncRead for TlsSocket {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>
    ) -> Poll<Result<(), io::Error>> {
        let mut this = match ready!(self.poll_accept(cx)) {
            Ok(this) => this,
            Err(err) => return Poll::Ready(Err(err))
        };
        match this.as_mut().project() {
            TlsSocketProj::Stream { fut } => {
                fut.poll_read(cx, buf)
            }
            _ => unreachable!()
        }
    }
}

impl AsyncWrite for TlsSocket {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8]
    ) -> Poll<Result<usize, io::Error>> {
        let mut this = match ready!(self.poll_accept(cx)) {
            Ok(this) => this,
            Err(err) => return Poll::Ready(Err(err))
        };
        match this.as_mut().project() {
            TlsSocketProj::Stream { fut } => {
                fut.poll_write(cx, buf)
            }
            _ => unreachable!()
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>
    ) -> Poll<Result<(), io::Error>> {
        let mut this = match ready!(self.poll_accept(cx)) {
            Ok(this) => this,
            Err(err) => return Poll::Ready(Err(err))
        };
        match this.as_mut().project() {
            TlsSocketProj::Stream { fut } => {
                fut.poll_flush(cx)
            }
            _ => unreachable!()
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>
    ) -> Poll<Result<(), io::Error>> {
        let mut this = match ready!(self.poll_accept(cx)) {
            Ok(this) => this,
            Err(err) => return Poll::Ready(Err(err))
        };
        match this.as_mut().project() {
            TlsSocketProj::Stream { fut } => {
                fut.poll_shutdown(cx)
            }
            _ => unreachable!()
        }
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
        let res = self.as_mut().project().sock.poll_read(cx, buf);
        res
    }
}

impl AsyncWrite for MyTcpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8]
    ) -> Poll<Result<usize, io::Error>> {
        let res = self.as_mut().project().sock.poll_write(cx, buf);
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

