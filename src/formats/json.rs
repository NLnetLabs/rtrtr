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

use rpki::resources::asn::Asn;
use rpki::resources::addr::{MaxLenError, MaxLenPrefix, Prefix};
use rpki::rtr::payload::{Aspa as AspaPayload, Payload, PayloadRef, RouteOrigin};
use rpki::rtr::pdu::{ProviderAsns, ProviderAsnsError};
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
    /// The list of ASPAs.
    aspas: Option<Vec<Aspa>>,
}

impl Set {
    /// Converts the JSON formatted data set into a payload set.
    pub fn into_payload(self) -> payload::Set {
        let mut res = payload::PackBuilder::empty();
        for item in self.roas {
            let _ = res.insert(item.into_payload());
        }
        if let Some(aspas) = self.aspas {
            for item in aspas {
                let _ = res.insert(item.into_payload());
            }
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

    fn try_from(
        JsonVrp {
            prefix,
            max_length,
            asn: JsonAsn(asn),
        }: JsonVrp,
    ) -> Result<Self, Self::Error> {
        MaxLenPrefix::new(prefix, Some(max_length)).map(|prefix| Vrp {
            payload: RouteOrigin::new(prefix, asn),
        })
    }
}

//------------ Aspa ----------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(try_from = "JsonAspa", into = "JsonAspa")]
struct Aspa {
    /// The payload of the ASPA.
    payload: AspaPayload,
}

impl Aspa {
    fn into_payload(self) -> Payload {
        Payload::Aspa(self.payload)
    }
}

impl TryFrom<JsonAspa> for Aspa {
    type Error = ProviderAsnsError;

    fn try_from(
        JsonAspa {
            providers,
            customer: JsonAsn(customer),
        }: JsonAspa,
    ) -> Result<Self, Self::Error> {
        let provider_asns =
            ProviderAsns::try_from_iter(providers.into_iter().map(|JsonAsn(asn)| asn))?;
        Ok(Self {
            payload: AspaPayload {
                customer,
                providers: provider_asns,
            },
        })
    }
}

//============ Serialization =================================================

//------------ JsonAsn -------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
struct JsonAsn(
    #[serde(
        serialize_with = "Asn::serialize_as_str",
        deserialize_with = "Asn::deserialize_from_any"
    )]
    Asn,
);

//------------ JsonVrp -------------------------------------------------------

/// A JSON formatted VRP.
///
/// This is a private helper type making the Serde impls easier.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct JsonVrp {
    /// The prefix member.
    prefix: Prefix,

    /// The ASN member.
    asn: JsonAsn,

    /// The max-length member.
    #[serde(rename = "maxLength")]
    max_length: u8,
}

impl From<Vrp> for JsonVrp {
    fn from(vrp: Vrp) -> Self {
        JsonVrp {
            prefix: vrp.payload.prefix.prefix(),
            asn: JsonAsn(vrp.payload.asn),
            max_length: vrp.payload.prefix.resolved_max_len(),
        }
    }
}

//------------ JsonAspa ------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize)]
struct JsonAspa {
    /// The customer ASN.
    ///
    /// For rpki-client compatibility, "customer_asid" is also supported
    #[serde(alias = "customer_asid")]
    customer: JsonAsn,
    /// The provider ASNs.
    providers: Vec<JsonAsn>,
}

impl From<Aspa> for JsonAspa {
    fn from(aspa: Aspa) -> Self {
        Self {
            customer: JsonAsn(aspa.payload.customer),
            providers: aspa
                .payload
                .providers
                .iter()
                .map(JsonAsn)
                .collect(),
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
#[derive(Clone, Copy, Debug, PartialEq)]
enum StreamState {
    /// `{`
    Start,

    /// `"roas": [`
    RoaStart,

    /// { .. }
    RoaBody,

    /// `"]"`
    RoaEnd,

    /// `"aspas": [`
    AspaStart,

    /// { .. }
    AspaBody,

    /// `']'`
    AspaEnd,

    /// `'}'`
    End,

    /// None
    Done,
}

impl OutputStream {
    /// Creates a new output stream for the given payload set.
    pub fn new(set: payload::Set) -> Self {
        OutputStream {
            iter: set.into_owned_iter(),
            state: StreamState::Start,
        }
    }
}

fn format_origin(origin: RouteOrigin, last: bool) -> Vec<u8> {
    format!(
        r#"    {{ "asn": "{}", "prefix": "{}", "maxLength": {}, "ta": "N/A" }}{}"#,
        origin.asn,
        origin.prefix.prefix(),
        origin.prefix.resolved_max_len(),
        if last { "\n" } else { ",\n" },
    ).into_bytes()
}

fn format_aspa(aspa: AspaPayload, last: bool) -> Vec<u8> {
    format!(
        r#"    {{ "customer": {}, "providers": {:?} }}{}"#,
        aspa.customer.into_u32(),
        aspa.providers.iter().map(|a| a.into_u32()).collect::<Vec<_>>(),
        if last { "\n" } else { ",\n" },
    ).into_bytes()
}

impl Iterator for OutputStream {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.state {
            StreamState::Start => {
                self.state = match self.iter.peek() {
                    Some(Payload::Origin(_)) => StreamState::RoaStart,
                    Some(Payload::Aspa(_)) => StreamState::AspaStart,
                    Some(Payload::RouterKey(_)) => StreamState::End,
                    None => StreamState::End,
                };
                Some(b"{\n".to_vec())
            }
            StreamState::RoaStart => {
                self.state = StreamState::RoaBody;
                Some(b"  \"roas\": [\n".to_vec())
            }
            StreamState::RoaBody => {
                let Some(PayloadRef::Origin(payload)) = self.iter.next() else {
                    unreachable!();
                };

                self.state = match self.iter.peek() {
                    Some(Payload::Origin(_)) => StreamState::RoaBody,
                    Some(Payload::Aspa(_)) => StreamState::RoaEnd,
                    Some(Payload::RouterKey(_)) => StreamState::RoaEnd,
                    None => StreamState::RoaEnd,
                };

                let last = self.state != StreamState::RoaBody;
                Some(format_origin(payload, last))
            }
            StreamState::RoaEnd => {
                self.state = match self.iter.peek() {
                    Some(Payload::Origin(_)) => unreachable!(),
                    Some(Payload::Aspa(_)) => StreamState::AspaStart,
                    Some(Payload::RouterKey(_)) => StreamState::End,
                    None => StreamState::End,
                };
                if self.state == StreamState::End {
                    Some(b"  ]\n".to_vec())
                } else {
                    Some(b"  ],\n".to_vec())
                }
            }
            StreamState::AspaStart => {
                self.state = StreamState::AspaBody;
                Some(b"  \"aspas\": [\n".to_vec())
            }
            StreamState::AspaBody => {
                let Some(PayloadRef::Aspa(payload)) = self.iter.next() else {
                    unreachable!();
                };
                let payload = payload.clone();

                self.state = match self.iter.peek() {
                    Some(Payload::Origin(_)) => unreachable!(),
                    Some(Payload::Aspa(_)) => StreamState::AspaBody,
                    Some(Payload::RouterKey(_)) => StreamState::AspaEnd,
                    None => StreamState::AspaEnd,
                };

                let last = self.state != StreamState::AspaBody;
                Some(format_aspa(payload, last))
            }
            StreamState::AspaEnd => {
                self.state = match self.iter.peek() {
                    Some(Payload::Origin(_)) => unreachable!(),
                    Some(Payload::Aspa(_)) => unreachable!(),
                    Some(Payload::RouterKey(_)) => StreamState::End,
                    None => StreamState::End,
                };
                if self.state == StreamState::End {
                    Some(b"  ]\n".to_vec())
                } else {
                    Some(b"  ],\n".to_vec())
                }
            }
            StreamState::End => {
                self.state = StreamState::Done;
                Some(b"}".to_vec())
            }
            StreamState::Done => None,
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

    #[test]
    fn serialize() {
        fn s(items: Vec<Payload>) -> String {
            let mut res = payload::PackBuilder::empty();
            for item in items {
                res.insert(item).unwrap();
            }
            let set: payload::Set = res.finalize().into();
            let output = OutputStream::new(set);
            let mut out = vec![];
            for item in output {
                out.extend_from_slice(&item);
            }
            String::from_utf8(out).unwrap()
        }

        assert_eq!(s(vec![]), "{\n}");
        assert_eq!(
            s(vec![
                Payload::Origin(RouteOrigin::new(MaxLenPrefix::new("fd00:1234::/32".parse().unwrap(), Some(48)).unwrap(), 42u32.into())),
            ]),
            "{\n  \"roas\": [\n    { \"asn\": \"AS42\", \"prefix\": \"fd00:1234::/32\", \"maxLength\": 48, \"ta\": \"N/A\" }\n  ]\n}"
        );
        assert_eq!(
            s(vec![
                Payload::Origin(RouteOrigin::new(MaxLenPrefix::new("fd00:1234::/32".parse().unwrap(), Some(48)).unwrap(), 42u32.into())),
                Payload::Origin(RouteOrigin::new(MaxLenPrefix::new("fd00:1235::/32".parse().unwrap(), Some(48)).unwrap(), 42u32.into())),
            ]),
            "{\n  \"roas\": [\n    { \"asn\": \"AS42\", \"prefix\": \"fd00:1234::/32\", \"maxLength\": 48, \"ta\": \"N/A\" },\n    { \"asn\": \"AS42\", \"prefix\": \"fd00:1235::/32\", \"maxLength\": 48, \"ta\": \"N/A\" }\n  ]\n}",
        );

        assert_eq!(
            s(vec![
                Payload::Aspa(AspaPayload { customer: 42u32.into(), providers: ProviderAsns::try_from_iter(vec![44u32.into(), 45u32.into()]).unwrap() }),
            ]),
            "{\n  \"aspas\": [\n    { \"customer\": 42, \"providers\": [44, 45] }\n  ]\n}",
        );
        assert_eq!(
            s(vec![
                Payload::Aspa(AspaPayload { customer: 42u32.into(), providers: ProviderAsns::try_from_iter(vec![44u32.into(), 45u32.into()]).unwrap() }),
                Payload::Aspa(AspaPayload { customer: 45u32.into(), providers: ProviderAsns::try_from_iter(vec![46u32.into(), 47u32.into()]).unwrap() }),
            ]),
            "{\n  \"aspas\": [\n    { \"customer\": 42, \"providers\": [44, 45] },\n    { \"customer\": 45, \"providers\": [46, 47] }\n  ]\n}",
        );

        assert_eq!(
            s(vec![
                Payload::Aspa(AspaPayload { customer: 42u32.into(), providers: ProviderAsns::try_from_iter(vec![44u32.into(), 45u32.into()]).unwrap() }),
                Payload::Origin(RouteOrigin::new(MaxLenPrefix::new("fd00:1234::/32".parse().unwrap(), Some(48)).unwrap(), 42u32.into())),
                Payload::Aspa(AspaPayload { customer: 45u32.into(), providers: ProviderAsns::try_from_iter(vec![46u32.into(), 47u32.into()]).unwrap() }),
                Payload::Origin(RouteOrigin::new(MaxLenPrefix::new("fd00:1235::/32".parse().unwrap(), Some(48)).unwrap(), 42u32.into())),
            ]),
            "{\n  \"roas\": [\n    { \"asn\": \"AS42\", \"prefix\": \"fd00:1234::/32\", \"maxLength\": 48, \"ta\": \"N/A\" },\n    { \"asn\": \"AS42\", \"prefix\": \"fd00:1235::/32\", \"maxLength\": 48, \"ta\": \"N/A\" }\n  ],\n  \"aspas\": [\n    { \"customer\": 42, \"providers\": [44, 45] },\n    { \"customer\": 45, \"providers\": [46, 47] }\n  ]\n}",
        );
    }
}

