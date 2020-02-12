//! Configuration.

use std::net::{SocketAddr, SocketAddrV4, Ipv4Addr};
use std::str::FromStr;
use clap::{App, Arg, ArgMatches};
use log::error;
use crate::server::ExitError;



//------------ Config --------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Config {
    /// The streams we are serving.
    pub streams: Vec<Stream>,
}

impl Config {
    pub fn config_args<'a: 'b, 'b>(app: App<'a, 'b>) -> App<'a, 'b> {
        app
        .arg(Arg::with_name("rtr-listen")
            .short("R")
            .long("rtr-listen")
            .takes_value(true)
            .value_name("ADDR")
            .help("address to listen to with plain RTR")
            .multiple(true)
            .number_of_values(1)
        )
        .arg(Arg::with_name("rtr-server")
            .short("r")
            .long("rtr-server")
            .takes_value(true)
            .value_name("ADDR")
            .help("address of a plain RTR server")
            .multiple(true)
            .number_of_values(1)
        )
    }

    pub fn from_arg_matches(
        matches: &ArgMatches,
    ) -> Result<Self, ExitError> {
        let mut res = Self::default();
        res.apply_arg_matches(matches)?;
        Ok(res)
    }

    fn apply_arg_matches(
        &mut self,
        matches: &ArgMatches,
    ) -> Result<(), ExitError> {
        let mut listen = Vec::new();
        match matches.values_of("rtr-listen") {
            Some(list) => {
                for value in list {
                    match SocketAddr::from_str(value) {
                        Ok(addr) => {
                            listen.push((addr, ServerProtocol::RtrTcp))
                        }
                        Err(_) => {
                            error!("Invalid value for rtr-listen: {}", value);
                            return Err(ExitError)
                        }
                    }
                }
            }
            None => {
                listen.push((
                    SocketAddr::V4(
                        SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 3323)
                    ),
                    ServerProtocol::RtrTcp,
                ))
            }
        }
        let listen = listen; // drop mut.

        let mut sources = Vec::new();
        match matches.values_of("rtr-source") {
            Some(list) => {
                for value in list {
                    match SocketAddr::from_str(value) {
                        Ok(some) => {
                            sources.push(Source::Server {
                                protocol: ServerProtocol::RtrTcp,
                                addr: some
                            })
                        }
                        Err(_) => {
                            error!("Invalid value for rtr-server: {}", value);
                            return Err(ExitError)
                        }
                    }
                }
            }
            None => {
                sources.push(Source::Server {
                    protocol: ServerProtocol::RtrTcp,
                    addr: SocketAddr::V4(
                        SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 3323)
                    )
                })
            }
        }
        let sources = sources; // drop mut

        self.streams = vec![
            Stream {
                name: "default".into(),
                listen,
                source: Source::Any {
                    sources,
                    random: false,
                }
            }
        ];

        Ok(())
    }
}


//------------ Stream --------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Stream {
    /// Name of the stream.
    pub name: String,

    /// Listen addresses and protocols.
    ///
    pub listen: Vec<(SocketAddr, ServerProtocol)>,

    /// source.
    pub source: Source,
}


//------------ ServerProtocol ------------------------------------------------

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub enum ServerProtocol {
    /// RTR over plain TCP.
    RtrTcp,
}


//------------ Source --------------------------------------------------------

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Source {
    /// A server.
    Server {
        /// The type of server.
        protocol: ServerProtocol,

        /// The network address of the server.
        addr: SocketAddr,
    },

    /// Any of multiple sources.
    Any {
        /// The list of sources.
        sources: Vec<Source>,

        /// Whether pick a random source
        ///
        /// If `false`, we always start with the first source, if `true`, we
        /// pick a source at random.
        random: bool,
    }

}

