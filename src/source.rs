//! Payload sources.

use std::net::SocketAddr;
use futures::future::{BoxFuture, FutureExt, pending};
use log::warn;
use rand::{thread_rng, Rng};
use rpki_rtr::client::Client as RtrClient;
use tokio::net::TcpStream;
use tokio::time::{Duration, delay_for};
use crate::config;
use crate::payload::StreamHandle;
use crate::server::ExitError;


//------------ Source --------------------------------------------------------

pub struct Source {
    choice: Choice,
    stream: StreamHandle,
}

impl Source {
    pub fn new(
        config: &config::Source,
        stream: StreamHandle,
    ) -> Result<Self, ExitError> {
        Ok(Source {
            choice: Choice::new(config)?,
            stream
        })
    }

    pub async fn run(self) {
        loop {
            let _ = self.choice.run(&self.stream).await;
            let _ = delay_for(Duration::from_secs(10)).await;
        }
    }
}


//------------ Choice --------------------------------------------------------

enum Choice {
    RtrTcp(RtrTcp),
    Any(Any),
    None,
}

impl Choice {
    fn new(
        config: &config::Source,
    ) -> Result<Self, ExitError> {
        match *config {
            config::Source::Server { protocol, addr } => {
                match protocol {
                    config::ServerProtocol::RtrTcp => {
                        RtrTcp::new(addr).map(Choice::RtrTcp)
                    }
                }
            }
            config::Source::Any { ref sources, random } => {
                if sources.is_empty() {
                    Ok(Choice::None)
                }
                else if sources.len() == 1 {
                    Self::new(&sources[0])
                }
                else {
                    Any::new(sources, random).map(Choice::Any)
                }
            }
        }
    }

    fn run<'a>(
        &'a self, stream: &'a StreamHandle
    ) -> BoxFuture<'a, Result<(), RunError>> {
        async move {
            match *self {
                Choice::RtrTcp(ref source) => source.run(stream).await,
                Choice::Any(ref source) => source.run(stream).await,
                Choice::None => pending().await,
            }
        }.boxed()
    }
}


//------------ Any -----------------------------------------------------------

struct Any {
    sources: Vec<Choice>,
    random: bool
}

impl Any {
    fn new(
        sources: &[config::Source], random: bool
    ) -> Result<Self, ExitError> {
        let mut res = Vec::new();
        for source in sources {
            res.push(Choice::new(source)?)
        };
        Ok(Any {
            sources: res,
            random
        })
    }

    async fn run(&self, stream: &StreamHandle) -> Result<(), RunError> {
        // Start at the end so the increment below makes us use the first.
        let mut current = self.sources.len() - 1;
        loop {
            if self.random {
                current = thread_rng().gen_range(0, self.sources.len());
            }
            else {
                current = (current + 1) % self.sources.len()
            }

            let mut errs = 0;
            loop {
                // The return values of a run are treated as follows:
                // 
                // Ok means upstream just terminated the connection cleanly.
                // We give the server a second (literally) and try to
                // reconnect.
                //
                // A transient error means something went wrong by we may want
                // to try again. We wait the retry timeout and the connect
                // again. But we only do that three times before moving on.
                //
                // On a fatal error we move to the next source right away.
                match self.sources[current].run(stream).await {
                    Ok(()) => {
                        delay_for(Duration::from_secs(1)).await;
                    }
                    Err(RunError::Transient)  => {
                        errs += 1;
                        if errs == 3 {
                            break;
                        }
                        delay_for(Duration::from_secs(
                            u64::from(stream.timing().retry)
                        )).await
                    }
                    Err(RunError::Fatal) => {
                        break
                    }
                }
            }
        }
    }
}


//------------ RtrTcp --------------------------------------------------------

struct RtrTcp {
    addr: SocketAddr,
}

impl RtrTcp {
    fn new(addr: SocketAddr) -> Result<Self, ExitError> {
        Ok(RtrTcp { addr })
    }

    async fn run(&self, stream: &StreamHandle) -> Result<(), RunError> {
        let sock = match TcpStream::connect(&self.addr).await {
            Ok(sock) => sock,
            Err(err) => {
                warn!(
                    "Failed to connect to RTR server {}: {}",
                    &self.addr, err
                );
                return Err(RunError::Fatal);
            }
        };
        RtrClient::new(sock, stream.clone(), None).run().await.map_err(|err| {
            warn!(
                "RTR connection to server {} dropped: {}",
                &self.addr, err
            );
            RunError::Transient
        })
    }
}


//------------ RunError -----------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum RunError {
    Transient,
    Fatal,
}

