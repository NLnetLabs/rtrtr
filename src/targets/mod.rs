/// The targets for RPKI data.
///
/// A target is anything that produces the final output from RPKI data.
/// Each target is connected to exactly one unit and constantly converts its
/// data set into some form of output.
///
/// This module contains all the different kinds of targets currently
/// available. It provides access to them via the enum `Target` that contains
/// all types as variants.
///
/// Targets can be created from configuration via serde deserialization. They
/// are started by spawning them into an async runtime and then just keep
/// running there.

//------------ Sub-modules ---------------------------------------------------
//
// These contain all the actual unit types grouped by shared functionality.
pub mod rtr;


//------------ Target --------------------------------------------------------

use serde_derive::Deserialize;
use crate::log::ExitError;


/// The fundamental entity for data processing.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum Target {
    #[serde(rename = "rtr")]
    RtrTcp(rtr::Tcp),
}

impl Target {
    pub async fn run(self, name: String) -> Result<(), ExitError> {
        match self {
            Target::RtrTcp(target) => target.run(name).await
        }
    }
}

