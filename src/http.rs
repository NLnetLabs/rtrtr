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
use bytes::Bytes;
use chrono::{DateTime, Utc};
use daemonbase::error::ExitError;
use futures_util::pin_mut;
use futures_util::stream::{Stream, StreamExt};
use http_body_util::{BodyExt, Empty, Full, StreamBody};
use http_body_util::combinators::BoxBody;
use hyper::{Method, StatusCode};
use hyper::body::{Body, Frame};
use hyper::http::response::Builder;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use log::{debug, error};
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use crate::metrics;
use crate::utils::http::format_http_date;


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
    /// via the configuration and spawns it onto the given `runtime`. The
    /// method should be run before `runtime` is started. It will
    /// synchronously create and bind all required sockets before returning.
    ///
    /// The server will use `metrics` to produce information on its metrics
    /// related endpoints.
    pub fn run(
        &self,
        metrics: metrics::Collection,
        resources: Resources,
        runtime: &Runtime,
    ) -> Result<(), ExitError> {
        // Bind and collect all listeners first so we can error out
        // if any of them fails.
        let mut listeners = Vec::new();
        for addr in &self.listen {
            // Binding needs to have happened before dropping privileges
            // during detach. So we do this here synchronously.
            let listener = match StdListener::bind(addr) {
                Ok(listener) => listener,
                Err(err) => {
                    error!("Fatal: error listening on {}: {}", addr, err);
                    return Err(ExitError::default());
                }
            };
            if let Err(err) = listener.set_nonblocking(true) {
                error!(
                    "Fatal: failed to set listener {} to non-blocking: {}.",
                    addr, err
                );
                return Err(ExitError::default());
            }
            debug!("HTTP server listening on {}", addr);
            listeners.push((listener, addr));
        }

        // Now spawn the listeners onto the runtime. This way, they will start
        // doing their thing as soon as the runtime is started.
        for (listener, addr) in listeners {
            runtime.spawn(
                Self::single_listener(
                    listener, *addr, metrics.clone(), resources.clone()
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
        addr: SocketAddr,
        metrics: metrics::Collection,
        resources: Resources,
    ) {
        let listener = match TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(err) => {
                error!("Error on HTTP listener: {}", err);
                return
            }
        };
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _addr)) => stream,
                Err(err) => {
                    error!("Fatal error in HTTP server {}: {}", addr, err);
                    break;
                }
            };
            let metrics = metrics.clone();
            let resources = resources.clone();
            tokio::task::spawn(async move {
                let _ = hyper_util::server::conn::auto::Builder::new(
                    TokioExecutor::new()
                ).serve_connection(
                    TokioIo::new(stream),
                    service_fn(move |req| {
                        let metrics = metrics.clone();
                        let resources = resources.clone();
                        async move {
                            Self::handle_request(
                                req, &metrics, &resources
                            ).await
                        }
                    })
                ).await;
            });
        }
    }

    /// Handles a single HTTP request.
    async fn handle_request(
        req: Request,
        metrics: &metrics::Collection,
        resources: &Resources,
    ) -> Result<Response, Infallible> {
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
    fn metrics(metrics: &metrics::Collection) -> Response {
        ResponseBuilder::ok()
        .content_type(ContentType::PROMETHEUS)
        .body(metrics.assemble(metrics::OutputFormat::Prometheus))
    }

    /// Produces the response for a call to the `/status` endpoint.
    fn status(metrics: &metrics::Collection) -> Response {
        ResponseBuilder::ok()
        .content_type(ContentType::TEXT)
        .body(
            metrics.assemble(metrics::OutputFormat::Plain)
        )
    }

    /// Produces the response for a Method Not Allowed error.
    fn method_not_allowed() -> Response {
        ResponseBuilder::method_not_allowed()
        .content_type(ContentType::TEXT)
        .body("Method Not Allowed")
    }

    /// Produces the response for a Not Found error.
    fn not_found() -> Response {
        ResponseBuilder::not_found()
        .content_type(ContentType::TEXT)
        .body("Not Found")
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
        &self, request: &Request,
    ) -> Option<Response> {
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
    /// If the processor feels responsible for the request, it should return
    /// some response. This can be an error response. Otherwise it should
    /// return `None`.
    fn process_request(
        &self, request: &Request
    ) -> Option<Response>;
}

impl<T: ProcessRequest> ProcessRequest for Arc<T> {
    fn process_request(
        &self, request: &Request
    ) -> Option<Response> {
        AsRef::<T>::as_ref(self).process_request(request)
    }
}

impl<F> ProcessRequest for F
where F: Fn(&Request) -> Option<Response> + Sync + Send {
    fn process_request(
        &self, request: &Request
    ) -> Option<Response> {
        (self)(request)
    }
}


//------------ Request -------------------------------------------------------

pub type Request = hyper::Request<hyper::body::Incoming>;


//------------ Response ------------------------------------------------------

pub type Response = hyper::Response<BoxBody<Bytes, Infallible>>;


//------------ ResponseBuilder -----------------------------------------------

#[derive(Debug)]
pub struct ResponseBuilder {
    builder: Builder,
}

impl ResponseBuilder {
    /// Creates a new builder with the given status.
    pub fn new(status: StatusCode) -> Self {
        ResponseBuilder { builder:  Builder::new().status(status) }
    }

    /// Creates a new builder for a 200 OK response.
    pub fn ok() -> Self {
        Self::new(StatusCode::OK)
    }

    /// Creates a new builder for a Service Unavailable response.
    pub fn service_unavailable() -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE)
    }

    /// Creates a new builder for a Bad Request response.
    pub fn bad_request() -> Self {
        Self::new(StatusCode::BAD_REQUEST)
    }

    /// Creates a new builder for a Not Found response.
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND)
    }

    /// Creates a new builder for a Not Modified response.
    pub fn not_modified() -> Self {
        Self::new(StatusCode::NOT_MODIFIED)
    }

    /// Creates a new builder for a Method Not Allowed response.
    pub fn method_not_allowed() -> Self {
        Self::new(StatusCode::METHOD_NOT_ALLOWED)
    }

    /// Creates a new builder for a Moved Permanently response.
    pub fn moved_permanently() -> Self {
        Self::new(StatusCode::MOVED_PERMANENTLY)
    }

    /// Adds the content type header.
    pub fn content_type(self, content_type: ContentType) -> Self {
        ResponseBuilder {
            builder: self.builder.header("Content-Type", content_type.0)
        }
    }

    /// Adds the ETag header.
    pub fn etag(self, etag: &str) -> Self {
        ResponseBuilder {
            builder: self.builder.header("ETag", etag)
        }
    }

    /// Adds the Last-Modified header.
    pub fn last_modified(self, last_modified: DateTime<Utc>) -> Self {
        ResponseBuilder {
            builder: self.builder.header(
                "Last-Modified",
                format_http_date(last_modified)
            )
        }
    }

    /// Adds the Location header.
    #[allow(dead_code)]
    pub fn location(self, location: &str) -> Self {
        ResponseBuilder {
            builder: self.builder.header(
                "Location",
                location
            )
        }
    }

    fn finalize<B>(self, body: B) -> Response
    where
        B: Body<Data = Bytes, Error = Infallible> + Send + Sync + 'static
    {
        self.builder.body(
            body.boxed()
        ).expect("broken HTTP response builder")
    }

    /// Finalizes the response by adding a body.
    pub fn body(self, body: impl Into<Bytes>) -> Response {
        self.finalize(Full::new(body.into()))
    }

    /// Finalizes the response by adding an empty body.
    pub fn empty(self) -> Response {
        self.finalize(Empty::new())
    }

    pub fn stream<S>(self, body: S) -> Response
    where
        S: Stream<Item = Bytes> + Send + Sync + 'static
    {
        self.finalize(
            StreamBody::new(body.map(|item| {
                Ok(Frame::data(item))
            }))
        )
    }
}


//------------ ContentType ---------------------------------------------------

#[derive(Clone, Debug)]
pub struct ContentType(&'static [u8]);

impl ContentType {
    pub const CSV: ContentType = ContentType(
        b"text/csv;charset=utf-8;header=present"
    );
    pub const JSON: ContentType = ContentType(b"application/json");
    pub const TEXT: ContentType = ContentType(b"text/plain;charset=utf-8");
    pub const PROMETHEUS: ContentType = ContentType(
        b"text/plain; version=0.0.4"
    );

    pub fn external(value: &'static [u8]) -> Self {
        ContentType(value)
    }
}


//------------ Wrapped sockets -----------------------------------------------

/// A TCP stream wrapped for use with Hyper.
struct HttpStream {
    sock: TcpStream,
}

impl AsyncRead for HttpStream {
    fn poll_read(
        mut self: Pin<&mut Self>, cx: &mut Context, buf: &mut ReadBuf
    ) -> Poll<Result<(), io::Error>> {
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

