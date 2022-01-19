//! Maintaining and outputting metrics.
//!
//! Metrics are operational data maintained by components that allow users to
//! understand what their instance of RTRTR is doing. Because they are updated
//! by components and printed by other components in different threads,
//! management is a bit tricky.
//!
//! Typically, all metrics of a component are kept in a single object that is
//! shared between that component and everything that could possibly output
//! metrics. We use atomic data types (such as `std::sync::atomic::AtomicU32`)
//! the keep and allow updating the actual values and keep the value behind an
//! arc for easy sharing.
//!
//! When a component is started, it registers its metrics object with a
//! metrics [`Collection`] it receives via its
//! [`Component`][crate::manager::Component].
//!
//! The object needs to implement the [`Source`] trait by appending all its
//! data to a [`Target`]. To make that task easier, the [`Metric`] type is
//! used to define all the properties of an individual metric. Values of this
//! type can be created as constants.

use std::fmt;
use std::sync::{Arc, Mutex, Weak};
use std::fmt::Write;
use arc_swap::ArcSwap;
use clap::{crate_name, crate_version};


//------------ Module Configuration ------------------------------------------

/// The application prefix to use in the names of Prometheus metrics.
const PROMETHEUS_PREFIX: &str = "rtrtr";


//------------ Collection ----------------------------------------------------

/// A collection of metrics sources.
///
/// This type provides a shared collection. I.e., if a value is cloned, both
/// clones will reference the same collection. Both will see newly
/// added sources.
///
/// Such new sources can be registered with the [`register`][Self::register]
/// method. A string with all the current values of all known sources can be
/// obtained via the [`assemble`][Self::assemble] method.
#[derive(Clone, Default)]
pub struct Collection {
    /// The currently registered sources.
    sources: Arc<ArcSwap<Vec<RegisteredSource>>>,

    /// A mutex to be held during registration of a new source.
    ///
    /// Updating `sources` is done by taking the existing sources,
    /// construct a new vec, and then swapping that vec into the arc. Because
    /// of this, updates cannot be done concurrently. The mutex guarantees
    /// that.
    register: Arc<Mutex<()>>,
}

impl Collection {
    /// Registers a new source with the collection.
    ///
    /// The name of the component registering the source is passed via `name`.
    /// The source itself is given as a weak pointer so that it gets dropped
    /// when the owning component terminates.
    pub fn register(&self, name: Arc<str>, source: Weak<dyn Source>) {
        let lock = self.register.lock().unwrap();
        let old_sources = self.sources.load();
        let mut new_sources = Vec::new();
        for item in old_sources.iter() {
            if item.source.strong_count() > 0 {
                new_sources.push(item.clone())
            }
        }
        new_sources.push(
            RegisteredSource { name, source }
        );
        new_sources.sort_by(|l, r| l.name.as_ref().cmp(r.name.as_ref()));
        self.sources.store(new_sources.into());
        drop(lock);
    }

    /// Assembles metrics output.
    ///
    /// Produces an output of all the sources in the collection in the given
    /// format and returns it as a string.
    pub fn assemble(&self, format: OutputFormat) -> String {
        let sources = self.sources.load();
        let mut target = Target::new(format);
        for item in sources.iter() {
            if let Some(source) = item.source.upgrade() {
                source.append(&item.name, &mut target)
            }
        }
        target.into_string()
    }
}


impl fmt::Debug for Collection {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let len = self.sources.load().len();
        write!(f, "Collection({} sources)", len)
    }
}


//------------ RegisteredSource ----------------------------------------------

/// All information on a source registered with a collection.
#[derive(Clone)]
struct RegisteredSource {
    /// The name of the component owning the source.
    name: Arc<str>,

    /// A weak pointer to the source.
    source: Weak<dyn Source>,
}


//------------ Source --------------------------------------------------------

/// A type producing some metrics.
///
/// All this type needs to be able to do is output its metrics.
pub trait Source: Send + Sync {
    /// Appends the metrics to the target.
    ///
    /// The unit name is provided so a source doesn’t need to keep it around.
    fn append(&self, unit_name: &str, target: &mut Target);
}

impl<T: Source> Source for Arc<T> {
    fn append(&self, unit_name: &str, target: &mut Target) {
        AsRef::<T>::as_ref(self).append(unit_name, target)
    }
}


//------------ Target --------------------------------------------------------

/// A target for outputting metrics.
///
/// A new target can be created via [`new`](Self::new), passing in the
/// requested output format. Individual metrics are appended to the target
/// via [`append`](Self::append) or the shortcut
/// [`append_simple`](Self::append_simple). Finally, when all metrics are
/// assembled, you can turn the target into a string of the output via
/// [`into_string`](Self::into_string).
#[derive(Clone, Debug)]
pub struct Target {
    /// The format of the assembled output.
    format: OutputFormat,

    /// The output assembled so far.
    target: String,
}

impl Target {
    /// Creates a new target.
    ///
    /// The target will produce output in the given format.
    pub fn new(format: OutputFormat) -> Self {
        let mut target = String::new();
        if matches!(format, OutputFormat::Plain) {
            target.push_str(
                concat!(
                    "version: ", crate_name!(), "/", crate_version!(), "\n"
                )
            );
        }
        Target { format, target }
    }

    /// Converts the target into a string with the assembled output.
    pub fn into_string(self) -> String {
        self.target
    }

    /// Appends metrics to the target.
    ///
    /// The method can append multiple metrics values at once via the closure.
    /// All values are, however, for the same metrics described by `metric`.
    /// If the values are for a specific component, it’s name is given via
    /// `unit_name`. If they are global, this can be left at `None`.
    pub fn append<F: FnOnce(&mut Records)>(
        &mut self,
        metric: &Metric,
        unit_name: Option<&str>,
        values: F,
    ) {
        if !self.format.supports_type(metric.metric_type) {
            return
        }

        if matches!(self.format, OutputFormat::Prometheus) {
            self.target.push_str("# HELP ");
            self.append_metric_name(metric, unit_name);
            self.target.push(' ');
            self.target.push_str(metric.help);
            self.target.push('\n');

            self.target.push_str("# TYPE ");
            self.append_metric_name(metric, unit_name);
            writeln!(&mut self.target, " {}", metric.metric_type).unwrap();
        }
        values(&mut Records { target: self, metric, unit_name })
    }

    /// Append a single metric value to the target.
    ///
    /// This is a shortcut version of [`append`](Self::append) when there is
    /// only a single value to be append for a metric. The metric is described
    /// by `metric`.  If the value is for a specific component, it’s name is
    /// given via `unit_name`. If they are global, this can be left at `None`.
    pub fn append_simple(
        &mut self,
        metric: &Metric,
        unit_name: Option<&str>,
        value: impl fmt::Display,
    ) {
        self.append(metric, unit_name, |records| {
            records.value(value)
        })
    }

    /// Constructs and appends the name of the given metric.
    fn append_metric_name(
        &mut self, metric: &Metric, unit_name: Option<&str>
    ) {
        match self.format {
            OutputFormat::Prometheus => {
                write!(&mut self.target,
                    "{}_{}_{}",
                    PROMETHEUS_PREFIX, metric.name, metric.unit
                ).unwrap();
            }
            OutputFormat::Plain => {
                match unit_name {
                    Some(unit) => {
                        write!(&mut self.target,
                            "{} {}", unit, metric.name
                        ).unwrap();
                    }
                    None => {
                        write!(&mut self.target,
                            "{}", metric.name
                        ).unwrap();
                    }
                }
            }
        }
    }
}


//------------ Records -------------------------------------------------------

/// Allows adding all values for an individual metric.
///
/// Values can either be simple, in which case they only consist of a value
/// and are appended via [`value`](Self::value), or they can be labelled, in
/// which case there are multiple values for a metric that are distinguished
/// via a set of labels. Such values are appended via
/// [`label_value`](Self::label_value).
pub struct Records<'a> {
    /// A reference to the target.
    target: &'a mut Target,

    /// A reference to the properties of the metric in question.
    metric: &'a Metric,

    /// An reference to the name of the component if any.
    unit_name: Option<&'a str>,
}

impl<'a> Records<'a> {
    /// Appends a simple value to the metrics target.
    ///
    /// The value is simply output via the `Display` trait.
    pub fn value(&mut self, value: impl fmt::Display) {
        match self.target.format {
            OutputFormat::Prometheus => {
                self.target.append_metric_name(
                    self.metric, self.unit_name
                );
                if let Some(unit_name) = self.unit_name {
                    write!(&mut self.target.target,
                        "{{component=\"{}\"}}", unit_name
                    ).unwrap();
                }
                writeln!(&mut self.target.target, " {}", value).unwrap()
            }
            OutputFormat::Plain => {
                self.target.append_metric_name(self.metric, self.unit_name);
                writeln!(&mut self.target.target, ": {}", value).unwrap()
            }
        }
    }

    /// Appends a single labelled value to the metrics target.
    ///
    /// The labels are a slice of pairs of strings with the first element the
    /// name of the label and the second the label value. The metrics value
    /// is simply printed via the `Display` trait.
    pub fn label_value(
        &mut self,
        labels: &[(&str, &str)],
        value: impl fmt::Display
    ) {
        match self.target.format {
            OutputFormat::Prometheus => {
                self.target.append_metric_name(self.metric, self.unit_name);
                self.target.target.push('{');
                let mut comma = false;
                if let Some(unit_name) = self.unit_name {
                    write!(&mut self.target.target,
                        "component=\"{}\"", unit_name
                    ).unwrap();
                    comma = true;
                }
                for (name, value) in labels {
                    if comma {
                        write!(&mut self.target.target,
                            ", {}=\"{}\"", name, value
                        ).unwrap();
                    }
                    else {
                        write!(&mut self.target.target,
                            "{}=\"{}\"", name, value
                        ).unwrap();
                        comma = true;
                    }
                }
                writeln!(&mut self.target.target, "}} {}", value).unwrap()
            }
            OutputFormat::Plain => {
                self.target.append_metric_name(self.metric, self.unit_name);
                for (name, value) in labels {
                    write!(&mut self.target.target,
                        " {}={}", name, value
                    ).unwrap();
                }
                writeln!(&mut self.target.target, ": {}", value).unwrap()
            }
        }
    }
}


//------------ OutputFormat --------------------------------------------------

/// The output format for metrics.
///
/// This is a non-exhaustive enum so that we can add additional metrics
/// without having to do breaking releases. Output for unknown formats should
/// be empty.
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub enum OutputFormat {
    /// Prometheus’ text-base exposition format.
    ///
    /// See <https://prometheus.io/docs/instrumenting/exposition_formats/>
    /// for details.
    Prometheus,

    /// Simple, human-readable plain-text output.
    Plain
}

impl OutputFormat {
    /// Returns whether the format supports non-numerical metrics.
    #[allow(clippy::match_like_matches_macro)]
    pub fn allows_text(self) -> bool {
        match self {
            OutputFormat::Prometheus => false,
            OutputFormat::Plain => true,
        }
    }

    /// Returns whether this output format supports this metric type.
    #[allow(clippy::match_like_matches_macro)]
    pub fn supports_type(self, metric: MetricType) -> bool {
        match (self, metric) {
            (OutputFormat::Prometheus, MetricType::Text) => false,
            _ => true
        }
    }
}


//------------ Metric --------------------------------------------------------

/// The properties of a metric.
pub struct Metric {
    /// The name of the metric.
    ///
    /// The final name written to the target will be composed of more than
    /// just this name according to the rules stipulated by the output format.
    pub name: &'static str,

    /// The help text for the metric.
    pub help: &'static str,

    /// The type of the metric.
    pub metric_type: MetricType,

    /// The unit of the metric.
    pub unit: MetricUnit,
}

impl Metric {
    /// Constructs a new metric from all values.
    ///
    /// This is a const function and can be used to construct associated
    /// constants.
    pub const fn new(
        name: &'static str, help: &'static str,
        metric_type: MetricType, unit: MetricUnit
    ) -> Self {
        Metric { name, help, metric_type, unit
        }
    }
}


//------------ MetricType ----------------------------------------------------

/// The type of a metric.
#[derive(Clone, Copy, Debug)]
pub enum MetricType {
    /// A monotonically increasing counter.
    ///
    /// Values can only increase or be reset to zero.
    Counter,

    /// A value that can go up and down.
    Gauge,

    /// A Prometheus-style histogram.
    Histogram,

    /// A Prometheus-style summary.
    Summary,

    /// A text metric.
    ///
    /// Metrics of this type are only output to output formats that allow
    /// text metrics.
    Text,
}

impl fmt::Display for MetricType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            MetricType::Counter => f.write_str("counter"),
            MetricType::Gauge => f.write_str("gauge"),
            MetricType::Histogram => f.write_str("histogram"),
            MetricType::Summary => f.write_str("summary"),
            MetricType::Text => f.write_str("text"),
        }
    }
}


//------------ MetricUnit ----------------------------------------------------

/// A unit of measure for a metric.
///
/// This determines what a value of 1 means.
#[derive(Clone, Copy, Debug)]
pub enum MetricUnit {
    Second,
    Celsius,
    Meter,
    Byte,
    Ratio,
    Volt,
    Ampere,
    Joule,
    Gram,

    /// Use this for counting things.
    Total,

    /// Use this for non-numerical metrics.
    Info,
}

impl fmt::Display for MetricUnit {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            MetricUnit::Second => f.write_str("seconds"),
            MetricUnit::Celsius => f.write_str("celsius"),
            MetricUnit::Meter => f.write_str("meters"),
            MetricUnit::Byte => f.write_str("bytes"),
            MetricUnit::Ratio => f.write_str("ratio"),
            MetricUnit::Volt => f.write_str("volts"),
            MetricUnit::Ampere => f.write_str("amperes"),
            MetricUnit::Joule => f.write_str("joules"),
            MetricUnit::Gram => f.write_str("grams"),
            MetricUnit::Total => f.write_str("total"),
            MetricUnit::Info => f.write_str("info"),
        }
    }
}

