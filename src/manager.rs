//! Controlling the entire operation.

use std::{fs, io};
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use clap::crate_version;
use daemonbase::error::Failed;
use log::error;
use serde::Deserialize;
use tokio::runtime;
use crate::{http, metrics};
use crate::comms::{Gate, GateAgent, Link};
use crate::config::{Config, ConfigFile, Marked};
use crate::targets::Target;
use crate::units::Unit;


//------------ HttpClientConfig ----------------------------------------------

#[derive(Clone, Debug, Default, Deserialize)]
pub struct HttpClientConfig {
    /// The proxy servers to use for outgoing HTTP requests.
    #[cfg(feature = "socks")]
    #[serde(default, rename = "http-proxies")]
    proxies: Vec<String>,

    /// Additional root certificates for outgoing HTTP requests.
    #[serde(default, rename = "http-root-certs")]
    root_certs: Vec<PathBuf>,

    /// The user agent string to use for outgoing HTTP requests.
    #[serde(rename = "http-user-agent")]
    user_agent: Option<String>,

    /// Local address to bind to for outgoing HTTP requests.
    #[serde(rename = "http-client-addr")]
    local_addr: Option<IpAddr>,
}


//------------ Component -----------------------------------------------------

/// Facilities available to all components.
///
/// Upon being started, every component receives one of these. It provides
/// access to information and services available to all components.
#[derive(Debug)]
pub struct Component {
    /// The component’s name.
    name: Arc<str>,

    /// The HTTP client config.
    http_config: Arc<HttpClientConfig>,

    /// A reference to the metrics collection.
    metrics: metrics::Collection,

    /// A reference to the HTTP resources collection.
    http_resources: http::Resources,
}

impl Component {
    /// Creates a new component from its, well, components.
    fn new(
        name: String,
        http_config: Arc<HttpClientConfig>,
        metrics: metrics::Collection,
        http_resources: http::Resources,
    ) -> Self {
        Component {
            name: name.into(), http_config, metrics, http_resources,
        }
    }

    /// Returns the name of the component.
    pub fn name(&self) -> &Arc<str> {
        &self.name
    }

    /// Register a metrics source.
    pub fn register_metrics(&mut self, source: Arc<dyn metrics::Source>) {
        self.metrics.register(self.name.clone(), Arc::downgrade(&source));
    }

    /// Register an HTTP resources.
    pub fn register_http_resource(
        &mut self, process: Arc<dyn http::ProcessRequest>
    ) {
        self.http_resources.register(Arc::downgrade(&process))
    }

    /// Creates a new HTTP client for the component.
    pub fn http_client(&self) -> Result<reqwest::ClientBuilder, String> {
        let mut builder = reqwest::Client::builder();
        
        #[cfg(feature = "socks")]
        for proxy in &self.http_config.proxies {
            let proxy = match reqwest::Proxy::all(proxy) {
                Ok(proxy) => proxy,
                Err(err) => {
                    return Err(format!(
                        "Invalid rrdp-proxy '{}': {}", proxy, err
                    ));
                }
            };
            builder = builder.proxy(proxy);
        }

        for path in &self.http_config.root_certs {
            builder = builder.add_root_certificate(
                Self::load_cert(path)?
            );
        }

        builder = builder.user_agent(
            match self.http_config.user_agent.as_ref() {
                Some(agent) => agent.as_str(),
                None => concat!("RTRTR ", crate_version!()),
            }
        );

        #[cfg(feature = "native-tls")]
        {
            builder = builder.use_native_tls();
        }

        if let Some(addr) = self.http_config.local_addr {
            builder = builder.local_address(addr)
        }

        Ok(builder)
    }

    /// Loads a WebPKI trusted certificate.
    fn load_cert(path: &Path) -> Result<reqwest::Certificate, String> {
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(err) => {
                return Err(format!(
                    "Cannot open rrdp-root-cert file '{}': {}'",
                    path.display(), err
                ));
            }
        };
        let mut data = Vec::new();
        if let Err(err) = io::Read::read_to_end(&mut file, &mut data) {
            return Err(format!(
                "Cannot read rrdp-root-cert file '{}': {}'",
                path.display(), err
            ));
        }
        reqwest::Certificate::from_pem(&data).map_err(|err| {
            format!(
                "Cannot decode rrdp-root-cert file '{}': {}'",
                path.display(), err
            )
        })
    }
}


//------------ Manager -------------------------------------------------------

/// A manager for components and auxiliary services.
#[derive(Default)]
pub struct Manager {
    /// The currently active units represented by agents to their gates.
    units: HashMap<String, GateAgent>,

    /// Gates for newly loaded, not yet spawned units.
    pending: HashMap<String, Gate>,

    /// The HTTP client config.
    http_config: Arc<HttpClientConfig>,

    /// The metrics collection maintained by this managers.
    metrics: metrics::Collection,

    /// The HTTP resources collection maintained by this manager.
    http_resources: http::Resources,
}


impl Manager {
    /// Creates a new manager.
    pub fn new(http_config: &HttpClientConfig) -> Self {
        Self {
            http_config: http_config.clone().into(),
            .. Default::default()
        }
    }

    /// Loads the given config file.
    ///
    /// Parses the given file as a TOML config file. All links to units
    /// referenced in the configuration are pre-connected.
    ///
    /// If there are any errors in the config file, they are logged as errors
    /// and a generic error is returned.
    ///
    /// If the method succeeds, you need to spawn all units and targets via
    /// the [`spawn`](Self::spawn) method.
    ///
    /// Returns both a new manager and the parsed config.
    pub fn load(
        file: ConfigFile
    ) -> Result<(Self, Config), Failed> {
        // Prepare the thread-local used to allow serde load the links in the
        // units and targets.
        GATES.with(|gates| {
            gates.replace(Some(Default::default()))
        });

        // Now load the config file.
        let config = match Config::from_toml(file.bytes(), file.dir()) {
            Ok(config) => config,
            Err(err) => {
                match file.path() {
                    Some(path) => error!("{}: {}", path.display(), err),
                    None => error!("{}", err)
                }
                return Err(Failed)
            }
        };

        let mut manager = Self::new(&config.http_client);

        // All entries in the thread-local that have a gate are new. They must
        // appear in config’s units or we have unresolved links.
        let gates = GATES.with(|gates| gates.replace(None) ).unwrap();
        let mut errs = Vec::new();
        for (name, load) in gates {
            if let Some(gate) = load.gate {
                if !config.units.units.contains_key(&name) {
                    for mut link in load.links {
                        link.resolve_config(&file);
                        errs.push(link.mark(
                            format!("unresolved link to unit '{}'", name)
                        ))
                    }
                }
                else {
                    manager.units.insert(name.clone(), load.agent);
                    manager.pending.insert(name, gate);
                }
            }
        }
        if !errs.is_empty() {
            for err in errs {
                error!("{}", err);
            }
            return Err(Failed)
        }

        Ok((manager, config))
    }

    /// Allows creating components and adding them to the manager.
    ///
    /// Because creating components that contain links requires some setup
    /// work, this has to happen inside a closure. Inside the closure, you
    /// can create units and targets and add them to the correct set. Once
    /// the closure returns, all of the units and targets are spawned onto
    /// the runtime represented by the `runtime` handle. If all of this
    /// succeeds, the method will return whatever the closure returned.
    pub fn add_components<F, T>(
        &mut self, runtime: &runtime::Handle, op: F
    ) -> Result<T, Failed>
    where
        F: FnOnce(&mut UnitSet, &mut TargetSet) -> T
    {
        GATES.with(|gates| {
            gates.replace(
                Some(self.units.iter().map(|(key, value)| {
                    (key.clone(), value.clone().into())
                }).collect())
            )
        });
        
        let mut units = UnitSet::new();
        let mut targets = TargetSet::new();
        let res = op(&mut units, &mut targets);

        // All entries in the thread-local that have a gate are new. They must
        // appear in unit set or we have unresolved links.
        let gates = GATES.with(|gates| gates.replace(None)).unwrap();
        let mut errs = Vec::new();
        for (name, load) in gates {
            if let Some(gate) = load.gate {
                if !units.units.contains_key(&name) {
                    errs.push(
                        format!("unresolved link to unit '{}'", name)
                    )
                }
                else {
                    self.units.insert(name.clone(), load.agent);
                    self.pending.insert(name, gate);
                }
            }
        }
        if !errs.is_empty() {
            for err in errs {
                error!("{}", err);
            }
            return Err(Failed)
        }

        self.spawn(&mut units, &mut targets, runtime);
        Ok(res)
    }

    /// Spawns all units and targets unto the given runtime.
    ///
    /// # Panics
    ///
    /// The method panics if the config hasn’t been successfully loaded via
    /// the same manager earlier.
    pub fn spawn(
        &mut self,
        units: &mut UnitSet,
        targets: &mut TargetSet,
        runtime: &runtime::Handle,
    ) {
        for (name, unit) in units.units.drain() {
            let gate = match self.pending.remove(&name) {
                Some(gate) => gate,
                None => {
                    error!("Unit {} is unused and will not be started.", name);
                    continue
                }
            };
            let controller = Component::new(
                name, self.http_config.clone(), self.metrics.clone(),
                self.http_resources.clone()
            );
            runtime.spawn(unit.run(controller, gate));
        }

        for (name, target) in targets.targets.drain() {
            let controller = Component::new(
                name, self.http_config.clone(), self.metrics.clone(),
                self.http_resources.clone()
            );
            runtime.spawn(target.run(controller));
        }

    }

    /// Returns a new reference to the manager’s metrics collection.
    pub fn metrics(&self) -> metrics::Collection {
        self.metrics.clone()
    }

    /// Returns a new reference the the HTTP resources collection.
    pub fn http_resources(&self) -> http::Resources {
        self.http_resources.clone()
    }
}


//------------ UnitSet -------------------------------------------------------

/// A set of units to be started.
#[derive(Default, Deserialize)]
#[serde(transparent)]
pub struct UnitSet {
    units: HashMap<String, Unit>,
}

impl UnitSet {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, unit: Unit) {
        self.units.insert(name.into(), unit);
    }
}


//------------ TargetSet -----------------------------------------------------

/// A set of targets to be started.
#[derive(Default, Deserialize)]
#[serde(transparent)]
pub struct TargetSet {
    targets: HashMap<String, Target>,
}

impl TargetSet {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, target: Target) {
        self.targets.insert(name.into(), target);
    }
}

//------------ LoadUnit ------------------------------------------------------

/// A unit referenced during loading.
struct LoadUnit {
    /// The gate of the unit.
    ///
    /// This is some only if the unit is newly created and has not yet been
    /// spawned onto a runtime.
    gate: Option<Gate>,

    /// A gate agent for the unit.
    agent: GateAgent,

    /// A list of location of links in the config.
    ///
    /// This is only used for generating errors if non-existing units are
    /// referenced in the config file.
    links: Vec<Marked<()>>,
}

impl Default for LoadUnit {
    fn default() -> Self {
        let (gate, agent) = Gate::new();
        LoadUnit {
            gate: Some(gate),
            agent,
            links: Vec::new()
        }
    }
}

impl From<GateAgent> for LoadUnit {
    fn from(agent: GateAgent) -> Self {
        LoadUnit {
            gate: None,
            agent,
            links: Vec::new()
        }
    }
}


//------------ Loading Links -------------------------------------------------

thread_local!(
    static GATES: RefCell<Option<HashMap<String, LoadUnit>>> = const {
        RefCell::new(None)
    }
);


/// Loads a link with the given name.
///
/// # Panics
///
/// This funtion panics if it is called outside of a run of
/// [`Manager::load`].
pub fn load_link(name: Marked<String>) -> Link {
    GATES.with(|gates| {
        let mut gates = gates.borrow_mut();
        let gates = gates.as_mut().unwrap();

        let mark = name.mark(());
        let name = name.into_inner();
        let unit = gates.entry(name).or_default();
        unit.links.push(mark);
        unit.agent.create_link()
    })
}

