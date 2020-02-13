
use std::process;
use std::net::SocketAddr;
use std::net::TcpListener as StdTcpListener;
use futures::future::pending;
use log::error;
use rpki_rtr::server::{Dispatch, DispatchRunner, Server as RtrServer};
use tokio::net::TcpListener;
use crate::config::{Config, ServerProtocol};
use crate::payload::StreamHandle;
use crate::source::Source;


//------------ Server --------------------------------------------------------

pub struct Server {
    config: Config,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Server {
            config
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub async fn run(self) -> Result<(), ExitError> {
        for stream_conf in &self.config.streams {
            let mut dispatch_rnr = DispatchRunner::new();
            let dispatch = dispatch_rnr.dispatch();
            let stream = StreamHandle::new(
                stream_conf.name.clone(),
                dispatch.get_sender()
            );
            let source = Source::new(
                &stream_conf.source,
                stream.clone(),
            )?;
            for &(addr, proto) in &stream_conf.listen {
                self.spawn_listener(
                    addr, proto, stream.clone(), dispatch.clone()
                )?
            }
            tokio::spawn(source.run());
            tokio::spawn(async move { dispatch_rnr.run().await });
        }
        pending().await
    }

    fn spawn_listener(
        &self,
        addr: SocketAddr, proto: ServerProtocol,
        stream: StreamHandle,
        dispatch: Dispatch
    ) -> Result<(), ExitError> {
        match proto {
            ServerProtocol::RtrTcp => self.spawn_rtr_tcp(addr, stream, dispatch)
        }
    }

    fn spawn_rtr_tcp(
        &self, addr: SocketAddr, stream: StreamHandle, dispatch: Dispatch,
    ) -> Result<(), ExitError> {
        let listener = match StdTcpListener::bind(addr) {
            Ok(listener) => listener,
            Err(err) => {
                error!("Can’t bind to {}: {}", addr, err);
                return Err(ExitError)
            }
        };
        let mut listener = match TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(err) => {
                error!("Fatal error listening on {}: {}", addr, err);
                return Err(ExitError)
            }
        };
        tokio::spawn(async move {
            let listener = listener.incoming();
            let server = RtrServer::new(listener, dispatch, stream);
            if server.run().await.is_err() {
                error!("Fatal error listening on {}.", addr);
            }
        });   
        Ok(())
    }
}


//------------ ExitError -----------------------------------------------------

pub struct ExitError;

impl ExitError {
    pub fn exit(self) -> ! {
        process::exit(1)
    }
}

