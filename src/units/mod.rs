//! Data processing units.
//!
//! RTRTR provides the means for flexible data processing through
//! interconnected entities called _units._ Each unit produces a constantly
//! updated data set. Other units can subscribe to updates from these sets.
//! Alternatively, they can produce their own data set from external input.
//! Different types of units exist that perform different tasks. They can
//! all be plugged together all kinds of ways.
//!
//! This module contains all the units currently available. It provides
//! access to them via a grand enum `Unit` that contains all unit types as
//! variants.
//!
//! Units can be created from configuration via serde deserialization. They
//! are started by spawning them into an async runtime and then just keep
//! running there.

//------------ Sub-modules ---------------------------------------------------
//
// These contain all the actual unit types grouped by shared functionality.
mod combine;
mod json;
mod rtr;
mod slurm;

//------------ Unit ----------------------------------------------------------

use serde::Deserialize;
use crate::comms::Gate;
use crate::manager::Component;

/// The fundamental entity for data processing.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum Unit {
    #[serde(rename = "any")]
    Any(combine::Any),

    #[serde(rename = "rtr")]
    RtrTcp(rtr::Tcp),

    #[serde(rename = "rtr-tls")]
    RtrTls(rtr::Tls),

    #[serde(rename = "json")]
    Json(json::Json),

    #[serde(rename = "slurm")]
    Slurm(slurm::LocalExceptions),
}

impl Unit {
    pub async fn run(
        self, component: Component, gate: Gate
    )  {
        let _ = match self {
            Unit::Any(unit) => unit.run(component, gate).await,
            Unit::RtrTcp(unit) => unit.run(component, gate).await,
            Unit::RtrTls(unit) => unit.run(component, gate).await,
            Unit::Json(unit) => unit.run(component, gate).await,
            Unit::Slurm(unit) => unit.run(component, gate).await,
        };
    }
}

