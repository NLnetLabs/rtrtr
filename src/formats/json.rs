//! RIPE NCC Validator/Cloudflare JSON format.
//!

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use rpki_rtr::payload::{Ipv4Prefix, Ipv6Prefix, Payload};
use serde::de;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
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
        let mut res = payload::SetBuilder::empty();
        for item in self.roas {
            let _ = res.insert(item.into_payload());
        }
        res.finalize()
    }
}


//------------ Vrp -----------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Vrp {
    asn: Asn,
    prefix: Prefix,

    #[serde(rename = "maxLength")]
    max_len: u8,
    ta: String,
}

impl Vrp {
    fn into_payload(self) -> Payload {
        match self.prefix.addr {
            IpAddr::V4(addr) => {
                Payload::V4(Ipv4Prefix {
                    prefix: addr,
                    prefix_len: self.prefix.prefix_len,
                    max_len: self.max_len,
                    asn: self.asn.0,
                })
            }
            IpAddr::V6(addr) => {
                Payload::V6(Ipv6Prefix {
                    prefix: addr,
                    prefix_len: self.prefix.prefix_len,
                    max_len: self.max_len,
                    asn: self.asn.0,
                })
            }
        }
    }
}


//------------ Asn -----------------------------------------------------------

#[derive(Clone, Debug)]
struct Asn(u32);

impl Serialize for Asn {
    fn serialize<S: Serializer>(
        &self, serializer: S
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_str(
            &format_args!("AS{}", self.0)
        )
    }
}

impl<'de> Deserialize<'de> for Asn {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D
    ) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Asn;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a string with an AS number")
            }

            fn visit_str<E: de::Error>(
                self, v: &str
            ) -> Result<Self::Value, E> {
                if v.len() < 3 {
                    return Err(E::invalid_value(
                        de::Unexpected::Str(v), &self
                    ))
                }
                if 
                    (v.as_bytes()[0] != b'a' && v.as_bytes()[0] != b'A')
                    || (v.as_bytes()[1] != b's' && v.as_bytes()[1] != b'S')
                {
                    return Err(E::invalid_value(
                        de::Unexpected::Str(v), &self
                    ))
                }
                match u32::from_str(&v[2..]) {
                    Ok(asn) => Ok(Asn(asn)),
                    Err(_) => {
                        Err(E::invalid_value(
                            de::Unexpected::Str(v), &self
                        ))
                    }
                }
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}


//------------ Prefix --------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Prefix {
    addr: IpAddr,
    prefix_len: u8,
}

impl Serialize for Prefix {
    fn serialize<S: Serializer>(
        &self, serializer: S
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_str(
            &format_args!("{}/{}", self.addr, self.prefix_len)
        )
    }
}

impl<'de> Deserialize<'de> for Prefix {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D
    ) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Prefix;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a string with a prefix in slash notation")
            }

            fn visit_str<E: de::Error>(
                self, v: &str
            ) -> Result<Self::Value, E> {
                let slash = match v.find('/') {
                    Some(slash) => slash,
                    None => {
                        return Err(E::invalid_value(
                            de::Unexpected::Str(v), &self
                        ))
                    }
                };
                let addr = match IpAddr::from_str(&v[..slash]) {
                    Ok(addr) => addr,
                    Err(_) => {
                        return Err(E::invalid_value(
                            de::Unexpected::Str(v), &self
                        ))
                    }
                };
                let prefix_len = match u8::from_str(&v[slash + 1..]) {
                    Ok(len) => len,
                    Err(_) => {
                        return Err(E::invalid_value(
                            de::Unexpected::Str(v), &self
                        ))
                    }
                };
                Ok(Prefix { addr, prefix_len })
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}


//------------ Metadata ------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Metadata {
    counts: usize,
    generated: u64,
    valid: u64,
    signature: String,

    #[serde(rename = "signatureDate")]
    signature_date: String,
}


//============ Output ========================================================

//------------ OutputStream --------------------------------------------------

pub struct OutputStream {
    iter: payload::SetIter,
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
    pub fn new(set: Arc<payload::Set>) -> Self {
        OutputStream {
            iter: set.into(),
            state: StreamState::Header,
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
                match self.iter.next() {
                    Some(Payload::V4(payload)) => {
                        self.state = StreamState::Body;
                        Some(format!(
                            "    {{ \"asn\": \"AS{}\", \"prefix\": \"{}/{}\", \
                            \"maxLength\": {}, \"ta\": \"N/A\" }}",
                            payload.asn,
                            payload.prefix,
                            payload.prefix_len,
                            payload.max_len,
                        ).into_bytes())
                    }
                    Some(Payload::V6(payload)) => {
                        self.state = StreamState::Body;
                        Some(format!(
                            "    {{ \"asn\": \"AS{}\", \"prefix\": \"{}/{}\", \
                            \"maxLength\": {}, \"ta\": \"N/A\" }}",
                            payload.asn,
                            payload.prefix,
                            payload.prefix_len,
                            payload.max_len,
                        ).into_bytes())
                    }
                    None => {
                        self.state = StreamState::Done;
                        Some(b"\n  ]\n}}".to_vec())
                    }
                }
            }
            StreamState::Body => {
                match self.iter.next() {
                    Some(Payload::V4(payload)) => {
                        Some(format!(
                            ",\n    \
                            {{ \"asn\": \"AS{}\", \"prefix\": \"{}/{}\", \
                            \"maxLength\": {}, \"ta\": \"N/A\" }}",
                            payload.asn,
                            payload.prefix,
                            payload.prefix_len,
                            payload.max_len,
                        ).into_bytes())
                    }
                    Some(Payload::V6(payload)) => {
                        Some(format!(
                            ",\n    \
                            {{ \"asn\": \"AS{}\", \"prefix\": \"{}/{}\", \
                            \"maxLength\": {}, \"ta\": \"N/A\" }}",
                            payload.asn,
                            payload.prefix,
                            payload.prefix_len,
                            payload.max_len,
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

    #[test]
    fn deserialize() {
        fn check_set(set: Set) {
            assert_eq!(set.roas.len(), 2);

            assert_eq!(set.roas[0].asn.0, 64512);
            assert_eq!(set.roas[0].prefix.addr, IpAddr::from([192,0,2,0]));
            assert_eq!(set.roas[0].prefix.prefix_len, 24);
            assert_eq!(set.roas[0].max_len, 24);
            assert_eq!(set.roas[0].ta, "ta");

            assert_eq!(set.roas[1].asn.0, 4200000000);
            assert_eq!(
                set.roas[1].prefix.addr,
                IpAddr::from_str("2001:DB8::").unwrap()
            );
            assert_eq!(set.roas[1].prefix.prefix_len, 32);
            assert_eq!(set.roas[1].max_len, 32);
            assert_eq!(set.roas[1].ta, "ta");
        }

        check_set(serde_json::from_slice::<Set>(
            include_bytes!("../../test-data/vrps.json")
        ).unwrap());
        check_set(serde_json::from_slice::<Set>(
            include_bytes!("../../test-data/vrps-metadata.json")
        ).unwrap());
    }
}

