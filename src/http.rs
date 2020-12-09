//! The HTTP server.
//!
//! Because with HTTP you can select what information you want per request,
//! we only have one HTTP server for the entire instance. HTTP targets will
//! provide their data via a specific base path within that server.
//!
//! Server configuration happens via the [`Server`] struct that normally is
//! part of the [`Config`](crate::config::Config).

use std::{fmt, io};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::net::TcpListener as StdListener;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll};
use arc_swap::ArcSwap;
use futures::pin_mut;
use hyper::{Body, Method, Request, Response, StatusCode};
use hyper::server::accept::Accept;
use hyper::service::{make_service_fn, service_fn};
use log::error;
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio::stream::Stream;
use crate::log::ExitError;
use crate::metrics;


//------------ Server --------------------------------------------------------

/// The configuration for the HTTP server.
#[derive(Clone, Deserialize)]
pub struct Server {
    /// The socket addresses to listen on.
    #[serde(rename = "http-listen")]
    listen: Vec<SocketAddr>,
}

impl Server {
    /// Runs the server.
    ///
    /// The method will start a new server listening on the sockets provided
    /// via the configuration and spawns it onto the given `runtime`.
    ///
    /// The server will use `metrics` to produce information on its metrics
    /// related endpoints.
    ///
    /// (In a future version, this function will also take an object
    /// reflecting additionally configured endpoints.)
    pub fn run(
        &self,
        metrics: metrics::Collection,
        resources: Resources,
        runtime: &Runtime,
    ) -> Result<(), ExitError> {
        let mut listeners = Vec::new();
        for addr in &self.listen {
            // Binding needs to have happened before dropping privileges
            // during detach. So we do this here synchronously.
            match StdListener::bind(addr) {
                Ok(listener) => listeners.push(listener),
                Err(err) => {
                    error!("Fatal: error listening on {}: {}", addr, err);
                    return Err(ExitError);
                }
            };
        }
        for listener in listeners {
            runtime.spawn(
                Self::single_listener(
                    listener, metrics.clone(), resources.clone()
                )
            );
        }
        Ok(())
    }
 
    /// Runs a single HTTP listener.
    ///
    /// Currently, this async function only resolves if the underlying
    /// listener encounters an error.
    async fn single_listener(
        listener: StdListener,
        metrics: metrics::Collection,
        resources: Resources,
    ) {
        let make_service = make_service_fn(|_conn| {
            let metrics = metrics.clone();
            let resources = resources.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req| {
                    let metrics = metrics.clone();
                    let resources = resources.clone();
                    async move {
                        Self::handle_request(req, &metrics, &resources).await
                    }
                }))
            }
        });
        let listener = match TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(err) => {
                error!("Error on HTTP listener: {}", err);
                return
            }
        };
        if let Err(err) = hyper::Server::builder(
            HttpAccept { sock: listener }
        ).serve(make_service).await {
            error!("HTTP server error: {}", err);
        }
    }

    /// Handles a single HTTP request.
    async fn handle_request(
        req: Request<Body>,
        metrics: &metrics::Collection,
        resources: &Resources,
    ) -> Result<Response<Body>, Infallible> {
        if *req.method() != Method::GET {
            return Ok(Self::method_not_allowed())
        }
        Ok(match req.uri().path() {
            "/metrics" => Self::metrics(metrics),
            "/status" => Self::status(metrics),
            _ => {
                match resources.process_request(&req) {
                    Some(response) => response,
                    None => Self::not_found()
                }
            }
        })
    }

    /// Produces the response for a call to the `/metrics` endpoint.
    fn metrics(metrics: &metrics::Collection) -> Response<Body> {
        Response::builder()
        .header("Content-Type", "text/plain; version=0.0.4")
        .body(
            metrics.assemble(metrics::OutputFormat::Prometheus).into()
        )
        .unwrap()
    }

    /// Produces the response for a call to the `/status` endpoint.
    fn status(metrics: &metrics::Collection) -> Response<Body> {
        Response::builder()
        .header("Content-Type", "text/plain")
        .body(
            metrics.assemble(metrics::OutputFormat::Plain).into()
        )
        .unwrap()
    }

    /// Produces the response for a Method Not Allowed error.
    fn method_not_allowed() -> Response<Body> {
        Response::builder()
        .status(StatusCode::METHOD_NOT_ALLOWED)
        .header("Content-Type", "text/plain")
        .body("Method Not Allowed".into())
        .unwrap()
    }

    /// Produces the response for a Not Found error.
    fn not_found() -> Response<Body> {
        Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain")
        .body("Not Found".into())
        .unwrap()
    }
}


//------------ Resources -----------------------------------------------------

/// A collection of HTTP resources to be served by the server.
///
/// This type provides a shared collection. I.e., if a value is cloned, both
/// clones will reference the same collection. Both will see newly
/// added resources.
///
/// Such new resources can be registered with the [`register`][Self::register]
/// method. An HTTP request can be processed using the
/// [`process_request`][Self::process_request] method.
#[derive(Clone, Default)]
pub struct Resources {
    /// The currently registered sources.
    sources: Arc<ArcSwap<Vec<RegisteredResource>>>,

    /// A mutex to be held during registration of a new source.
    ///
    /// Updating `sources` is done by taking the existing sources,
    /// construct a new vec, and then swapping that vec into the arc. Because
    /// of this, updates cannot be done concurrently. The mutex guarantees
    /// that.
    register: Arc<Mutex<()>>,
}

impl Resources {
    /// Registers a new processor with the collection.
    ///
    /// The processor is given as a weak pointer so that it gets dropped
    /// when the owning component terminates.
    pub fn register(&self, process: Weak<dyn ProcessRequest>) {
        let lock = self.register.lock().unwrap();
        let old_sources = self.sources.load();
        let mut new_sources = Vec::new();
        for item in old_sources.iter() {
            if item.process.strong_count() > 0 {
                new_sources.push(item.clone())
            }
        }
        new_sources.push(
            RegisteredResource { process }
        );
        self.sources.store(new_sources.into());
        drop(lock);
    }

    /// Processes an HTTP request.
    ///
    /// Returns some response if any of the registered processors actually
    /// processed the particular request or `None` otherwise.
    pub fn process_request(
        &self, request: &Request<Body>
    ) -> Option<Response<Body>> {
        let sources = self.sources.load();
        for item in sources.iter() {
            if let Some(process) = item.process.upgrade() {
                if let Some(response) = process.process_request(request) {
                    return Some(response)
                }
            }
        }
        None
    }
}


impl fmt::Debug for Resources {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let len = self.sources.load().len();
        write!(f, "Resource({} processors)", len)
    }
}


//------------ RegisteredResource --------------------------------------------

/// All information on a resource registered with a collection.
#[derive(Clone)]
struct RegisteredResource {
    /// A weak pointer to the resource’s processor.
    process: Weak<dyn ProcessRequest>,
}


//------------ ProcessRequest ------------------------------------------------

/// A type that can process an HTTP request.
pub trait ProcessRequest: Send + Sync {
    /// Processes an HTTP request.
    ///
    /// If the processor feels responsible for the reuqest, it should return
    /// some response. This can be an error response. Otherwise it should
    /// return `None`.
    fn process_request(
        &self, request: &Request<Body>
    ) -> Option<Response<Body>>;
}

impl<T: ProcessRequest> ProcessRequest for Arc<T> {
    fn process_request(
        &self, request: &Request<Body>
    ) -> Option<Response<Body>> {
        AsRef::<T>::as_ref(self).process_request(request)
    }
}

impl<F> ProcessRequest for F
where F: Fn(&Request<Body>) -> Option<Response<Body>> + Sync + Send {
    fn process_request(
        &self, request: &Request<Body>
    ) -> Option<Response<Body>> {
        (self)(request)
    }
}


//------------ Wrapped sockets -----------------------------------------------

/// A TCP listener wrapped for use with Hyper.
struct HttpAccept {
    sock: TcpListener,
}

impl Accept for HttpAccept {
    type Conn = HttpStream;
    type Error = io::Error;

    fn poll_accept(
        mut self: Pin<&mut Self>,
        cx: &mut Context
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let sock = &mut self.sock;
        pin_mut!(sock);
        sock.poll_next(cx).map(|sock| sock.map(|sock| sock.map(|sock| {
            HttpStream {
                sock,
            }
        })))
    }
}


/// A TCP stream wrapped for use with Hyper.
struct HttpStream {
    sock: TcpStream,
}

impl AsyncRead for HttpStream {
    fn poll_read(
        mut self: Pin<&mut Self>, cx: &mut Context, buf: &mut [u8]
    ) -> Poll<Result<usize, io::Error>> {
        let sock = &mut self.sock;
        pin_mut!(sock);
        sock.poll_read(cx, buf)
    }
}

impl AsyncWrite for HttpStream {
    fn poll_write(
        mut self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]
    ) -> Poll<Result<usize, io::Error>> {
        let sock = &mut self.sock;
        pin_mut!(sock);
        sock.poll_write(cx, buf)
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

