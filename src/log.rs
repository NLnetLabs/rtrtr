//! Logging.
//!
//! This module provides facilities to set up logging based on a configuration
//! via [`LogConfig`].
//!
//! The module also provides two error types [`Failed`] and [`ExitError`] that
//! indicate that error information has been logged and a consumer can just
//! return quietly.
use std::{fmt, io, process};
use std::convert::TryFrom;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use clap::{App, Arg, ArgMatches};
use log::{error, LevelFilter, Log};
use serde::Deserialize;


//------------ LogConfig -----------------------------------------------------

/// Logging configuration.
#[derive(Deserialize)]
pub struct LogConfig {
    /// Where to log to?
    #[serde(default)]
    pub log_target: LogTarget,

    /// If logging to a file, use this file.
    ///
    /// This isn’t part of `log_target` for deserialization reasons.
    #[serde(default)]
    pub log_file: PathBuf,

    /// The syslog facility when logging to syslog.
    ///
    /// This isn’t part of `log_target` for deserialization reasons.
    #[cfg(unix)]
    #[serde(default)]
    pub log_facility: LogFacility,

    /// The minimum log level to actually log.
    #[serde(default)]
    pub log_level: LogFilter,
}

impl LogConfig {
    /// Configures a clap app with the options for logging.
    pub fn config_args<'a: 'b, 'b>(app: App<'a, 'b>) -> App<'a, 'b> {
        app
        .arg(Arg::with_name("verbose")
             .short("v")
             .long("verbose")
             .multiple(true)
             .help("Log more information, twice or thrice for even more")
        )
        .arg(Arg::with_name("quiet")
             .short("q")
             .long("quiet")
             .multiple(true)
             .conflicts_with("verbose")
             .help("Log less information, twice for no information")
        )
        .arg(Arg::with_name("syslog")
             .long("syslog")
             .help("Log to syslog")
        )
        .arg(Arg::with_name("syslog-facility")
             .long("syslog-facility")
             .takes_value(true)
             .default_value("daemon")
             .help("Facility to use for syslog logging")
        )
        .arg(Arg::with_name("logfile")
             .long("logfile")
             .takes_value(true)
             .value_name("PATH")
             .help("Log to this file")
        )
    }

    /// Update the logging configuration from command line arguments.
    ///
    /// This should be called after the configuration file has been loaded.
    pub fn update_with_arg_matches(
        &mut self,
        matches: &ArgMatches,
        cur_dir: &Path,
    ) -> Result<(), Failed> {
        // log_level
        for _ in 0..matches.occurrences_of("verbose") {
            self.log_level.increase()
        }
        for _ in 0..matches.occurrences_of("quiet") {
            self.log_level.decrease()
        }

        self.apply_log_matches(matches, cur_dir)?;

        Ok(())
    }

    /// Applies the logging-specific command line arguments to the config.
    ///
    /// This is the Unix version that also considers syslog as a valid
    /// target.
    #[cfg(unix)]
    fn apply_log_matches(
        &mut self,
        matches: &ArgMatches,
        cur_dir: &Path,
    ) -> Result<(), Failed> {
        if matches.is_present("syslog") {
            self.log_target = LogTarget::Syslog;
            if let Some(value) = 
                Self::from_str_value_of(matches, "syslog-facility")?
            {
                self.log_facility = value
            }
        }
        else if let Some(file) = matches.value_of("logfile") {
            if file == "-" {
                self.log_target = LogTarget::Stderr
            }
            else {
                self.log_target = LogTarget::File;
                self.log_file = cur_dir.join(file);
            }
        }
        Ok(())
    }

    /// Applies the logging-specific command line arguments to the config.
    ///
    /// This is the non-Unix version that does not use syslog.
    #[cfg(not(unix))]
    #[allow(clippy::unnecessary_wraps)]
    fn apply_log_matches(
        &mut self,
        matches: &ArgMatches,
        cur_dir: &Path,
    ) -> Result<(), Failed> {
        if let Some(file) = matches.value_of("logfile") {
            if file == "-" {
                self.log_target = LogTarget::Stderr
            }
            else {
                self.log_target = LogTarget::File;
                self.log_file = cur_dir.join(file);
            }
        }
        Ok(())
    }


    /// Try to convert a string encoded value.
    ///
    /// This helper function just changes error handling. Instead of returning
    /// the actual conversion error, it logs it as an invalid value for entry
    /// `key` and returns the standard error.
    #[allow(dead_code)] // unused on Windows
    fn from_str_value_of<T>(
        matches: &ArgMatches,
        key: &str
    ) -> Result<Option<T>, Failed>
    where T: FromStr, T::Err: fmt::Display {
        match matches.value_of(key) {
            Some(value) => {
                match T::from_str(value) {
                    Ok(value) => Ok(Some(value)),
                    Err(err) => {
                        error!(
                            "Invalid value for {}: {}.", 
                            key, err
                        );
                        Err(Failed)
                    }
                }
            }
            None => Ok(None)
        }
    }

    /// Initialize logging.
    ///
    /// All diagnostic output of RTRTR is done via logging, never to
    /// stderr directly. Thus, it is important to initalize logging before
    /// doing anything else that may result in such output. This function
    /// does exactly that. It sets a maximum log level of `warn`, leading
    /// only printing important information, and directs all logging to
    /// stderr.
    pub fn init_logging() -> Result<(), ExitError> {
        log::set_max_level(log::LevelFilter::Warn);
        if let Err(err) = log_reroute::init() {
            eprintln!("Failed to initialize logger: {}.\nAborting.", err);
            return Err(ExitError)
        };
        let dispatch = fern::Dispatch::new()
            .level(log::LevelFilter::Error)
            .chain(io::stderr())
            .into_log().1;
        log_reroute::reroute_boxed(dispatch);
        Ok(())
    }

    /// Switches logging to the configured target.
    ///
    /// Once the configuration has been successfully loaded, logging should
    /// be switched to whatever the user asked for via this method.
    #[allow(unused_variables)] // for cfg(not(unix))
    pub fn switch_logging(&self, daemon: bool) -> Result<(), Failed> {
        let logger = match self.log_target {
            #[cfg(unix)]
            LogTarget::Default => {
                if daemon {
                    self.syslog_logger()?
                }
                else {
                    self.stderr_logger(false)
                }
            }
            #[cfg(not(unix))]
            LogTarget::Default => {
                self.stderr_logger(daemon)
            }
            #[cfg(unix)]
            LogTarget::Syslog => {
                self.syslog_logger()?
            }
            LogTarget::Stderr => {
                self.stderr_logger(daemon)
            }
            LogTarget::File => {
                self.file_logger()?
            }
        };
        log_reroute::reroute_boxed(logger);
        log::set_max_level(self.log_level.0);
        Ok(())
    }

    /// Creates a syslog logger and configures correctly.
    #[cfg(unix)]
    fn syslog_logger(
        &self
    ) -> Result<Box<dyn Log>, Failed> {
        let process = std::env::current_exe().ok().and_then(|path|
            path.file_name()
                .and_then(std::ffi::OsStr::to_str)
                .map(ToString::to_string)
        ).unwrap_or_else(|| String::from("routinator"));
        let pid = unsafe { libc::getpid() };
        let formatter = syslog::Formatter3164 {
            facility: self.log_facility.0,
            hostname: None,
            process,
            pid
        };
        let logger = syslog::unix(formatter.clone()).or_else(|_| {
            syslog::tcp(formatter.clone(), ("127.0.0.1", 601))
        }).or_else(|_| {
            syslog::udp(formatter, ("127.0.0.1", 0), ("127.0.0.1", 514))
        });
        match logger {
            Ok(logger) => Ok(Box::new(syslog::BasicLogger::new(logger))),
            Err(err) => {
                error!("Cannot connect to syslog: {}", err);
                Err(Failed)
            }
        }
    }

    /// Creates a stderr logger.
    ///
    /// If we are in daemon mode, we add a timestamp to the output.
    fn stderr_logger(&self, daemon: bool) -> Box<dyn Log>{
        self.fern_logger(daemon).chain(io::stderr()).into_log().1
    }

    /// Creates a file logger using the file provided by `path`.
    fn file_logger(&self) -> Result<Box<dyn Log>, Failed> {
        let file = match fern::log_file(&self.log_file) {
            Ok(file) => file,
            Err(err) => {
                error!(
                    "Failed to open log file '{}': {}",
                    self.log_file.display(), err
                );
                return Err(Failed)
            }
        };
        Ok(self.fern_logger(true).chain(file).into_log().1)
    }

    /// Creates and returns a fern logger.
    fn fern_logger(&self, timestamp: bool) -> fern::Dispatch {
        let mut res = fern::Dispatch::new();
        if timestamp {
            res = res.format(|out, message, _record| {
                out.finish(format_args!(
                    "{} {} {}",
                    chrono::Local::now().format("[%Y-%m-%d %H:%M:%S]"),
                    _record.module_path().unwrap_or(""),
                    message
                ))
            });
        }
        res = res
            .level(self.log_level.0)
            .level_for("rustls", LevelFilter::Error);
        if self.log_level.0 == LevelFilter::Debug {
            res = res
                .level_for("tokio_reactor", LevelFilter::Info)
                .level_for("hyper", LevelFilter::Info)
                .level_for("reqwest", LevelFilter::Info)
                .level_for("h2", LevelFilter::Info);
        }
        res
    }
}



//------------ LogTarget -----------------------------------------------------

/// The target to log to.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub enum LogTarget {
    /// Use the system default.
    #[serde(rename = "default")]
    Default,

    /// Syslog.
    #[cfg(unix)]
    #[serde(rename = "syslog")]
    Syslog,

    /// Stderr.
    #[serde(rename = "stderr")]
    Stderr,

    /// A file.
    #[serde(rename = "file")]
    File
}


//--- Default

impl Default for LogTarget {
    fn default() -> Self {
        LogTarget::Default
    }
}


//------------ LogFacility ---------------------------------------------------

#[cfg(unix)]
#[derive(Deserialize)]
#[serde(try_from = "String")]
pub struct LogFacility(syslog::Facility);

#[cfg(unix)]
impl Default for LogFacility {
    fn default() -> Self {
        LogFacility(syslog::Facility::LOG_DAEMON)
    }
}

#[cfg(unix)]
impl TryFrom<String> for LogFacility {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        syslog::Facility::from_str(&value).map(LogFacility).map_err(|_| {
            format!("unknown syslog facility {}", &value)
        })
    }
}

#[cfg(unix)]
impl FromStr for LogFacility {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        syslog::Facility::from_str(s).map(LogFacility).map_err(|_| {
            "unknown facility"
        })
    }
}


//------------ LogFilter -----------------------------------------------------

#[derive(Deserialize)]
#[serde(try_from = "String")]
pub struct LogFilter(log::LevelFilter);

impl LogFilter {
    pub fn increase(&mut self) {
        use log::LevelFilter::*;

        self.0 = match self.0 {
            Off => Error,
            Error => Warn,
            Warn => Info,
            Info => Debug,
            Debug => Trace,
            Trace => Trace,
        }
    }

    pub fn decrease(&mut self) {
        use log::LevelFilter::*;

        self.0 = match self.0 {
            Off => Off,
            Error => Off,
            Warn => Error,
            Info => Warn,
            Debug => Info,
            Trace => Debug,
        }
    }
}

impl Default for LogFilter {
    fn default() -> Self {
        LogFilter(log::LevelFilter::Warn)
    }
}

impl TryFrom<String> for LogFilter {
    type Error = log::ParseLevelError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        log::LevelFilter::from_str(&value).map(LogFilter)
    }
}



//------------ Failed --------------------------------------------------------

/// An error happened that has been logged.
///
/// This is a marker type that can be used in results to indicate that if an
/// error happend, it has been logged and doesn’t need further treatment.
#[derive(Clone, Copy, Debug)]
pub struct Failed;


//------------ ExitError -----------------------------------------------------

/// An error happened that should cause the process to exit.
pub struct ExitError;

impl ExitError {
    /// Exits the process.
    pub fn exit(self) -> ! {
        process::exit(1)
    }
}

impl From<Failed> for ExitError {
    fn from(_: Failed) -> ExitError {
        ExitError
    }
}

