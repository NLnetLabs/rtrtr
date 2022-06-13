//! The semi-standard JSON format for validated RPKI data.
//!
//! There are multiple slightly different flavours around. This
//! implementation tries to be able to read all of them and produces the
//! flavour used by Routinator (because of course).
//!
//! Specifically, we expect the JSON file to be an object with one member
//! called `"roa"` which contains a list of object. Each object represents
//! one VRP and contains one member called `"prefix"` containing the prefix
//! as a string in ‘slash notation,’ one member called `"asn"` with the AS
//! number as either an integer or a string with or without the `AS` prefix,
//! and one member called `maxLength` with the max length as an integer.
//!
//! Additional members are allowed both in the top-level object and the VRP
//! objects. They are simply ignored.
//!
//! When creating a JSON file, this minimal format will be used. The ASN will
//! be represented as a string with the `AS` prefix.

use std::convert::TryFrom;
use routecore::asn::Asn;
use routecore::addr::{MaxLenError, MaxLenPrefix, Prefix};
use rpki::rtr::payload::{RouteOrigin, Payload};
use rpki::rtr::server::PayloadSet;
use serde::{Deserialize, Serialize};
use crate::payload;


//============ Input =========================================================

//------------ Set -----------------------------------------------------------

/// The content of a JSON formatted data set.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Set {
    /// The list of VRPs.
    roas: Vec<Vrp>,
}

impl Set {
    /// Converts the JSON formatted data set into a payload set.
    pub fn into_payload(self) -> payload::Set {
        let mut res = payload::PackBuilder::empty();
        for item in self.roas {
            let _ = res.insert(item.into_payload());
        }
        res.finalize().into()
    }
}


//------------ Vrp -----------------------------------------------------------

/// The content of a JSON formatted VRP.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(try_from = "JsonVrp", into = "JsonVrp")]
struct Vrp {
    /// The payload of the VRP.
    payload: RouteOrigin,
}

impl Vrp {
    /// Converts the JSON VRP into regular payload.
    fn into_payload(self) -> Payload {
        Payload::Origin(self.payload)
    }
}

impl TryFrom<JsonVrp> for Vrp {
    type Error = MaxLenError;

    fn try_from(json: JsonVrp) -> Result<Self, Self::Error> {
        MaxLenPrefix::new(json.prefix, Some(json.max_length)).map(|prefix| {
            Vrp {
                payload: RouteOrigin::new(prefix, json.asn),
            }
        })
    }
}


//============ Serialization =================================================


//------------ JsonVrp -------------------------------------------------------

/// A JSON formatted VRP.
///
/// This is a private helper type making the Serde impls easier.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct JsonVrp {
    /// The prefix member.
    prefix: Prefix,
    
    /// The ASN member.
    #[serde(
        serialize_with = "Asn::serialize_as_str",
        deserialize_with = "Asn::deserialize_from_any",
    )]
    asn: Asn,

    /// The max-length member.
    #[serde(rename = "maxLength")]
    max_length: u8,
}

impl From<Vrp> for JsonVrp {
    fn from(vrp: Vrp) -> Self {
        JsonVrp {
            prefix: vrp.payload.prefix.prefix(),
            asn: vrp.payload.asn,
            max_length: vrp.payload.prefix.resolved_max_len(),
        }
    }
}


//============ Output ========================================================

//------------ OutputStream --------------------------------------------------

/// A stream of JSON formatted output.
pub struct OutputStream {
    /// The iterator over the payload set.
    iter: payload::OwnedSetIter,

    /// The current stream state.
    state: StreamState,
}

/// The state of the stream.
#[derive(Clone, Copy, Debug)]
enum StreamState {
    /// We need to write the header next.
    Header,

    /// We need to write the first element next.
    First,

    /// We need to write more elements.
    Body,

    /// We are done!
    Done
}

impl OutputStream {
    /// Creates a new output stream for the given payload set.
    pub fn new(set: payload::Set) -> Self {
        OutputStream {
            iter: set.into_owned_iter(),
            state: StreamState::Header,
        }
    }

    /// Returns the next route origin in the payload set.
    pub fn next_origin(&mut self) -> Option<RouteOrigin> {
        loop {
            match self.iter.next() {
                Some(Payload::Origin(value)) => return Some(*value),
                None => return None,
                _ => {}
            }
        }
    }
}

impl Iterator for OutputStream {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.state {
            StreamState::Header => {
                self.state = StreamState::First;
                Some(b"{\n  \"roas\": [\n".to_vec())
            }
            StreamState::First => {
                match self.next_origin() {
                    Some(payload) => {
                        self.state = StreamState::Body;
                        Some(format!(
                            "    {{ \"asn\": \"{}\", \"prefix\": \"{}\", \
                            \"maxLength\": {}, \"ta\": \"N/A\" }}",
                            payload.asn,
                            payload.prefix.prefix(),
                            payload.prefix.resolved_max_len(),
                        ).into_bytes())
                    }
                    None => {
                        self.state = StreamState::Done;
                        Some(b"\n  ]\n}".to_vec())
                    }
                }
            }
            StreamState::Body => {
                match self.next_origin() {
                    Some(payload) => {
                        Some(format!(
                            ",\n    \
                            {{ \"asn\": \"{}\", \"prefix\": \"{}\", \
                            \"maxLength\": {}, \"ta\": \"N/A\" }}",
                            payload.asn,
                            payload.prefix.prefix(),
                            payload.prefix.resolved_max_len(),
                        ).into_bytes())
                    }
                    None => {
                        self.state = StreamState::Done;
                        Some(b"\n  ]\n}".to_vec())
                    }
                }
            }
            StreamState::Done => {
                None
            }
        }
    }
}


//============ Testing =======================================================

#[cfg(test)]
mod test {
    use super::*;
    use std::net::IpAddr;
    use std::str::FromStr;

    #[test]
    fn deserialize() {
        fn check_set(set: Set) {
            assert_eq!(set.roas.len(), 2);

            assert_eq!(set.roas[0].payload.asn, 64512.into());
            assert_eq!(
                set.roas[0].payload.prefix.addr(),
                IpAddr::from([192,0,2,0])
            );
            assert_eq!(set.roas[0].payload.prefix.prefix_len(), 24);
            assert_eq!(set.roas[0].payload.prefix.max_len(), Some(24));

            assert_eq!(set.roas[1].payload.asn, 4200000000.into());
            assert_eq!(
                set.roas[1].payload.prefix.addr(),
                IpAddr::from_str("2001:DB8::").unwrap()
            );
            assert_eq!(set.roas[1].payload.prefix.prefix_len(), 32);
            assert_eq!(set.roas[1].payload.prefix.max_len(), Some(32));
        }

        check_set(serde_json::from_slice::<Set>(
            include_bytes!("../../test-data/vrps.json")
        ).unwrap());
        check_set(serde_json::from_slice::<Set>(
            include_bytes!("../../test-data/vrps-metadata.json")
        ).unwrap());
        check_set(serde_json::from_slice::<Set>(
            include_bytes!("../../test-data/vrps.rpki-client.json")
        ).unwrap());
    }
}

