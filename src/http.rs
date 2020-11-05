/// The HTTP server.

use std::io;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::net::TcpListener as StdListener;
use std::pin::Pin;
use std::task::{Context, Poll};
use futures::pin_mut;
use hyper::{Body, Method, Request, Response, StatusCode};
use hyper::server::accept::Accept;
use hyper::service::{make_service_fn, service_fn};
use log::error;
use serde_derive::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio::stream::Stream;
use crate::log::ExitError;
use crate::metrics;


//------------ Server --------------------------------------------------------

#[derive(Clone, Deserialize)]
pub struct Server {
    #[serde(rename = "http-listen")]
    listen: Vec<SocketAddr>,
}

impl Server {
    pub fn run(
        &self,
        metrics: metrics::Collection,
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
                Self::single_listener(listener, metrics.clone())
            );
        }
        Ok(())
    }
 
    async fn single_listener(
        listener: StdListener,
        metrics: metrics::Collection,
    ) {
        let make_service = make_service_fn(|_conn| {
            let metrics = metrics.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req| {
                    let metrics = metrics.clone();
                    async move { Self::handle_request(req, &metrics).await }
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

    async fn handle_request(
        req: Request<Body>,
        metrics: &metrics::Collection
    ) -> Result<Response<Body>, Infallible> {
        if *req.method() != Method::GET {
            return Ok(Self::method_not_allowed())
        }
        Ok(match req.uri().path() {
            "/metrics" => Self::metrics(metrics),
            "/status" => Self::status(metrics),
            _ => Self::not_found()
        })
    }

    fn metrics(metrics: &metrics::Collection) -> Response<Body> {
        Response::builder()
        .header("Content-Type", "text/plain; version=0.0.4")
        .body(
            metrics.assemble(metrics::OutputFormat::Prometheus).into()
        )
        .unwrap()
    }

    fn status(metrics: &metrics::Collection) -> Response<Body> {
        Response::builder()
        .header("Content-Type", "text/plain")
        .body(
            metrics.assemble(metrics::OutputFormat::Plain).into()
        )
        .unwrap()
    }

    fn method_not_allowed() -> Response<Body> {
        Response::builder()
        .status(StatusCode::METHOD_NOT_ALLOWED)
        .header("Content-Type", "text/plain")
        .body("Method Not Allowed".into())
        .unwrap()
    }

    fn not_found() -> Response<Body> {
        Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain")
        .body("Not Found".into())
        .unwrap()
    }
}


//------------ Wrapped sockets -----------------------------------------------

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

