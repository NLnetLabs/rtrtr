//! Metrics.

use std::{fmt, iter};
use std::sync::{Arc, Weak};
use std::fmt::Write;
use arc_swap::ArcSwap;
use clap::{crate_name, crate_version};


//------------ Module Configuration ------------------------------------------

const PROMETHEUS_PREFIX: &str = "rtrtr";


//------------ Collection ----------------------------------------------------

#[derive(Clone, Default)]
pub struct Collection {
    sources: ArcSwap<Vec<RegisteredSource>>,
}

impl Collection {
    pub fn register(&self, name: Arc<str>, source: Weak<dyn Source>) {
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
        self.sources.store(new_sources.into())
    }

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

#[derive(Clone)]
struct RegisteredSource {
    name: Arc<str>,
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

#[derive(Clone, Debug)]
pub struct Target {
    format: OutputFormat,
    target: String,
}

impl Target {
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

    pub fn into_string(self) -> String {
        self.target
    }

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

    fn append_metric_name(
        &mut self, metric: &Metric, unit_name: Option<&str>
    ) {
        match self.format {
            OutputFormat::Prometheus => {
                match unit_name {
                    Some(unit) => {
                        write!(&mut self.target,
                            "{}_{}_{}_{}",
                            PROMETHEUS_PREFIX, unit, metric.name, metric.unit
                        ).unwrap();
                    }
                    None => {
                        write!(&mut self.target,
                            "{}_{}_{}",
                            PROMETHEUS_PREFIX, metric.name, metric.unit
                        ).unwrap();
                    }
                }
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

pub struct Records<'a> {
    target: &'a mut Target,
    metric: &'a Metric,
    unit_name: Option<&'a str>,
}

impl<'a> Records<'a> {
    pub fn value(&mut self, value: impl fmt::Display) {
        match self.target.format {
            OutputFormat::Prometheus => {
                self.target.append_metric_name(self.metric, self.unit_name);
                writeln!(&mut self.target.target, " {}", value).unwrap()
            }
            OutputFormat::Plain => {
                self.target.append_metric_name(self.metric, self.unit_name);
                writeln!(&mut self.target.target, ": {}", value).unwrap()
            }
        }
    }

    pub fn label_value(
        &mut self,
        labels: &[(&str, &str)],
        value: impl fmt::Display
    ) {
        match self.target.format {
            OutputFormat::Prometheus => {
                self.target.append_metric_name(self.metric, self.unit_name);
                self.target.target.push('{');
                for ((name, value), comma) in
                    labels.iter().zip(
                        iter::once(false).chain(iter::repeat(true))
                    )
                {
                    if comma {
                        write!(&mut self.target.target,
                            ", {}=\"{}\"", name, value
                        ).unwrap();
                    }
                    else {
                        write!(&mut self.target.target,
                            "{}=\"{}\"", name, value
                        ).unwrap();
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
    /// See https://prometheus.io/docs/instrumenting/exposition_formats/
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

pub struct Metric {
    pub name: &'static str,
    pub help: &'static str,
    pub metric_type: MetricType,
    pub unit: MetricUnit,
}

impl Metric {
    pub const fn new(
        name: &'static str, help: &'static str,
        metric_type: MetricType, unit: MetricUnit
    ) -> Self {
        Metric { name, help, metric_type, unit
        }
    }
}


//------------ MetricType ----------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub enum MetricType {
    Counter,
    Gauge,
    Histogram,
    Summary,
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
    Total,
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

