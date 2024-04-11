//! Configuration.
//!
//! RTRTR is configured through a single TOML configuration file. We use
//! [serde] to deserialize this file into the [`Config`] struct provided by
//! this module. This struct also provides the facilities to load the config
//! file referred to in command line options.

use std::{borrow, error, fmt, fs, io, ops};
use std::path::Path;
use std::sync::Arc;
use clap::{Args as _, FromArgMatches};
use daemonbase::logging;
use daemonbase::config::ConfigPath;
use daemonbase::error::Failed;
use serde::Deserialize;
use toml::Spanned;
use crate::http;
use crate::manager::{HttpClientConfig, Manager, TargetSet, UnitSet};


//------------ Config --------------------------------------------------------

/// The complete RTRTR configuration.
///
/// All configuration is available via public fields.
///
/// The associated function [`init`](Self::init) should be called first thing
/// as it initializes the operational environment such as logging. Thereafter,
/// [`config_args`](Self::config_args) can be used to configure a clap app to
/// be able to pick up the path to the configuration file.
/// [`from_arg_matches`](Self::from_arg_matches) will then load the file
/// referenced in the command line and, upon success, return the config.
#[derive(Deserialize)]
pub struct Config {
    /// The set of configured units.
    pub units: UnitSet,
    
    /// The set of configured targets.
    pub targets: TargetSet,

    /// The logging configuration.
    #[serde(flatten)]
    pub log: logging::Config,

    /// The HTTP server configuration.
    #[serde(flatten)]
    pub http: http::Server,

    /// The HTTP client configuration.
    #[serde(flatten)]
    pub http_client: HttpClientConfig,
}

impl Config {
    /// Adds the basic arguments to a Clap command.
    ///
    /// Returns the command with the arguments added.
    pub fn config_args(app: clap::Command) -> clap::Command {
        Args::augment_args(app)
    }

    /// Creates a configuration from a bytes slice with TOML data.
    pub fn from_toml(
        slice: &str, base_dir: Option<impl AsRef<Path>>,
    ) -> Result<Self, toml::de::Error> {
        if let Some(ref base_dir) = base_dir {
            ConfigPath::set_base_path(base_dir.as_ref().into())
        }
        let res = toml::de::from_str(slice);
        ConfigPath::clear_base_path();
        res
    }

    /// Loads the configuration based on command line options provided.
    ///
    /// The `matches` must be the result of getting argument matches from a
    /// clap app previously configured with
    /// [`config_args`](Self::config_args). Otherwise, the function is likely
    /// to panic.
    ///
    /// The current path needs to be provided to be able to deal with relative
    /// paths.
    ///
    /// Returns both a manager and the config.
    pub fn from_arg_matches(
        matches: &clap::ArgMatches,
    ) -> Result<(Manager, Self), Failed> {
        let args = Args::from_arg_matches(
            matches
        ).expect("bug in command line arguments parser");
        let conf = match ConfigFile::load(&args.config) {
            Ok(conf) => conf,
            Err(err) => {
                eprintln!(
                    "Failed to read config file '{}': {}",
                    args.config.display(),
                    err
                );
                return Err(Failed)
            }
        };
        let (manager, mut config) = Manager::load(conf)?;
        config.log.apply_args(&args.log);
        Ok((manager, config))
    }
}


//------------ Args ----------------------------------------------------------

#[derive(clap::Parser)]
pub struct Args {
    #[arg(short, long)]
    pub config: ConfigPath,

    #[command(flatten)]
    pub log: logging::Args,
}


//------------ Source --------------------------------------------------------

/// Description of the source of configuration.
///
/// This type is used for error reporting. It can refer to a configuration
/// file or an interactive session.
///
/// File names are kept behind and arc and thus this type can be cloned
/// cheaply.
#[derive(Clone, Debug)]
struct Source {
    /// The optional path of a config file.
    ///
    /// If this in `None`, the source is an interactive session.
    path: Option<Arc<Path>>,
}

impl<'a, T: AsRef<Path>> From<&'a T> for Source {
    fn from(path: &'a T) -> Source {
        Source {
            path: Some(path.as_ref().into())
        }
    }
}


//------------ LineCol -------------------------------------------------------

/// A pair of a line and column number.
///
/// This is used for error reporting.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct LineCol {
    pub line: usize,
    pub col: usize
}


//------------ Marked --------------------------------------------------------

/// A value marked with its source location.
///
/// This wrapper is used when data needs to be resolved after parsing has
/// finished. In this case, we need information about the source location
/// to be able to produce meaningful error messages.
#[derive(Clone, Debug, Deserialize)]
#[serde(from = "Spanned<T>")]
pub struct Marked<T> {
    value: T,
    index: usize,
    source: Option<Source>,
    pos: Option<LineCol>,
}

impl<T> Marked<T> {
    /// Resolves the position for the given config file.
    pub fn resolve_config(&mut self, config: &ConfigFile) {
        self.source = Some(config.source.clone());
        self.pos = Some(config.resolve_pos(self.index));
    }

    /// Returns a reference to the value.
    pub fn as_inner(&self) -> &T {
        &self.value
    }

    /// Converts the marked value into is unmarked value.
    pub fn into_inner(self) -> T {
        self.value
    }

    /// Marks some other value with this value’s position.
    pub fn mark<U>(&self, value: U) -> Marked<U> {
        Marked {
            value,
            index: self.index,
            source: self.source.clone(),
            pos: self.pos,
        }
    }

    /// Formats the mark for displaying.
    pub fn format_mark(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let source = self.source.as_ref().and_then(|source|
            source.path.as_ref()
        );
        match (source, self.pos) {
            (Some(source), Some(pos)) => {
                write!(f, "{}:{}:{}", source.display(), pos.line, pos.col)
            }
            (Some(source), None) => write!(f, "{}", source.display()),
            (None, Some(pos)) => write!(f, "{}:{}", pos.line, pos.col),
            (None, None) => Ok(())
        }
    }
}


//--- From

impl<T> From<T> for Marked<T> {
    fn from(src: T) -> Marked<T> {
        Marked {
            value: src,
            index: 0,
            source: None, pos: None,
        }
    }
}

impl<T> From<Spanned<T>> for Marked<T> {
    fn from(src: Spanned<T>) -> Marked<T> {
        Marked {
            index: src.span().start,
            value: src.into_inner(),
            source: None, pos: None,
        }
    }
}


//--- Deref, AsRef, Borrow

impl<T> ops::Deref for Marked<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.as_inner()
    }
}

impl<T> AsRef<T> for Marked<T> {
    fn as_ref(&self) -> &T {
        self.as_inner()
    }
}

impl<T> borrow::Borrow<T> for Marked<T> {
    fn borrow(&self) -> &T {
        self.as_inner()
    }
}


//--- Display and Error

impl<T: fmt::Display> fmt::Display for Marked<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.format_mark(f)?;
        write!(f, ": {}", self.value)
    }
}

impl<T: error::Error> error::Error for Marked<T> { }


//------------ ConfigFile ----------------------------------------------------

/// A config file.
#[derive(Debug)]
pub struct ConfigFile {
    /// The source for this file.
    source: Source,

    /// The data of this file.
    bytes: String,

    /// The start indexes of lines.
    ///
    /// The start index of the first line is in `line_start[0]` and so on.
    line_starts: Vec<usize>,
}

impl ConfigFile {
    /// Load a config file from disk.
    pub fn load(path: &impl AsRef<Path>) -> Result<Self, io::Error> {
        fs::read_to_string(path).map(|bytes| {
            ConfigFile {
                source: path.into(),
                line_starts: bytes.split('\n').fold(
                    vec![0], |mut starts, slice| {
                        starts.push(
                            // slice doesn’t include the \n
                            starts.last().unwrap() + slice.len() + 1
                        );
                        starts
                    }
                ),
                bytes,
            }
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.source.path.as_ref().map(|path| path.as_ref())
    }

    pub fn dir(&self) -> Option<&Path> {
        self.source.path.as_ref().and_then(|path| path.parent())
    }

    pub fn bytes(&self) -> &str {
        &self.bytes
    }

    fn resolve_pos(&self, pos: usize) -> LineCol {
        let line = self.line_starts.iter().enumerate().find_map(|(i, start)|
            if *start > pos {
                Some(i)
            }
            else {
                None
            }
        ).unwrap_or(self.line_starts.len());
        let line = line - 1;
        let col = pos - self.line_starts[line];
        LineCol { line, col }
    }
}


//------------ ConfigError --------------------------------------------------

/// An error occurred during parsing of a configuration file.
#[derive(Clone, Debug)]
pub struct ConfigError {
    err: toml::de::Error,
    pos: Marked<()>,
}

impl ConfigError {
    pub fn new(err: toml::de::Error, file: &ConfigFile) -> Self {
        ConfigError {
            pos: Marked {
                value: (),
                index: 0,
                source: Some(file.source.clone()),
                pos: err.span().map(|range| {
                    file.resolve_pos(range.start)
                }),
            },
            err,
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.pos.format_mark(f)?;
        write!(f, ": {}", self.err)
    }
}

impl error::Error for ConfigError { }

