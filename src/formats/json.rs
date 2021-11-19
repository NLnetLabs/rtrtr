//! RIPE NCC Validator/Cloudflare/rpki-client JSON format.
//!

use std::convert::TryFrom;
use routecore::asn::Asn;
use routecore::addr::{MaxLenError, MaxLenPrefix, Prefix};
use rpki::rtr::payload::{RouteOrigin, Payload};
use rpki::rtr::server::PayloadSet;
use serde::{Deserialize, Serialize};
use crate::payload;


//============ Input =========================================================

//------------ Set -----------------------------------------------------------

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Set {
    metadata: Option<Metadata>,
    roas: Vec<Vrp>,
}

impl Set {
    pub fn into_payload(self) -> payload::Set {
        let mut res = payload::PackBuilder::empty();
        for item in self.roas {
            let _ = res.insert(item.into_payload());
        }
        res.finalize().into()
    }
}


//------------ Vrp -----------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(try_from = "JsonVrp", into = "JsonVrp")]
struct Vrp {
    payload: RouteOrigin,
}

impl Vrp {
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

#[derive(Clone, Debug, Deserialize, Serialize)]
struct JsonVrp {
    prefix: Prefix,
    
    #[serde(
        serialize_with = "Asn::serialize_as_str",
        deserialize_with = "Asn::deserialize_from_any",
    )]
    asn: Asn,

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


//------------ Metadata ------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Metadata {
}


//============ Output ========================================================

//------------ OutputStream --------------------------------------------------

pub struct OutputStream {
    iter: payload::OwnedSetIter,
    state: StreamState,
}

#[derive(Clone, Copy, Debug)]
enum StreamState {
    Header,
    First,
    Body,
    Done
}

impl OutputStream {
    pub fn new(set: payload::Set) -> Self {
        OutputStream {
            iter: set.into_owned_iter(),
            state: StreamState::Header,
        }
    }

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
                Some(b"{{\n  \"roas\": [\n".to_vec())
            }
            StreamState::First => {
                match self.next_origin() {
                    Some(payload) => {
                        self.state = StreamState::Body;
                        Some(format!(
                            "    {{ \"asn\": \"AS{}\", \"prefix\": \"{}/{}\", \
                            \"maxLength\": {}, \"ta\": \"N/A\" }}",
                            payload.asn,
                            payload.prefix.prefix(),
                            payload.prefix.prefix_len(),
                            payload.prefix.resolved_max_len(),
                        ).into_bytes())
                    }
                    None => {
                        self.state = StreamState::Done;
                        Some(b"\n  ]\n}}".to_vec())
                    }
                }
            }
            StreamState::Body => {
                match self.next_origin() {
                    Some(payload) => {
                        Some(format!(
                            ",\n    \
                            {{ \"asn\": \"AS{}\", \"prefix\": \"{}/{}\", \
                            \"maxLength\": {}, \"ta\": \"N/A\" }}",
                            payload.asn,
                            payload.prefix.prefix(),
                            payload.prefix.prefix_len(),
                            payload.prefix.resolved_max_len(),
                        ).into_bytes())
                    }
                    None => {
                        self.state = StreamState::Done;
                        Some(b"\n  ]\n}}".to_vec())
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

