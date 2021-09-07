//! RIPE NCC Validator/Cloudflare JSON format.
//!

use std::fmt;
use rpki::payload::addr::{MaxLenPrefix, Prefix};
use rpki::payload::rtr::{PrefixOrigin, Payload};
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

#[derive(Clone, Debug)]
struct Vrp {
    payload: PrefixOrigin,
    ta: String,
}

impl Vrp {
    fn into_payload(self) -> Payload {
        Payload::Origin(self.payload)
    }
}

//--- Deserialize and Serialize
//

impl<'de> Deserialize<'de> for Vrp {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D
    ) -> Result<Self, D::Error> {
        use serde::de;

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        enum Fields { Prefix, Asn, MaxLength, Ta }

        struct StructVisitor;

        impl<'de> de::Visitor<'de> for StructVisitor {
            type Value = Vrp;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("VRP struct")
            }

            fn visit_map<V: de::MapAccess<'de>>(
                self, mut map: V
            ) -> Result<Self::Value, V::Error> {
                let mut prefix = None;
                let mut asn = None;
                let mut max_len = None;
                let mut ta = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Fields::Prefix => {
                            if prefix.is_some() {
                                return Err(
                                    de::Error::duplicate_field("prefix")
                                );
                            }
                            prefix = Some(map.next_value()?);
                        }
                        Fields::Asn => {
                            if asn.is_some() {
                                return Err(
                                    de::Error::duplicate_field("asn")
                                );
                            }
                            asn = Some(map.next_value()?);
                        }
                        Fields::MaxLength => {
                            if max_len.is_some() {
                                return Err(
                                    de::Error::duplicate_field("maxLength")
                                );
                            }
                            max_len = Some(map.next_value()?);
                        }
                        Fields::Ta => {
                            if ta.is_some() {
                                return Err(
                                    de::Error::duplicate_field("ta")
                                );
                            }
                            ta = Some(map.next_value()?);
                        }
                    }
                }

                let prefix: Prefix = prefix.ok_or_else(|| {
                    de::Error::missing_field("prefix")
                })?;
                let asn = asn.ok_or_else(|| {
                    de::Error::missing_field("asn")
                })?;
                let max_len = max_len.ok_or_else(|| {
                    de::Error::missing_field("maxLength")
                })?;
                let ta = ta.ok_or_else(|| {
                    de::Error::missing_field("ta")
                })?;

                let prefix = MaxLenPrefix::new(prefix, max_len).map_err(
                    de::Error::custom
                )?;

                Ok(Vrp { payload: PrefixOrigin::new(prefix, asn), ta })
            }
        }

        const FIELDS: &[&str] = &[
            "prefix", "asn", "maxLength", "ta"
        ];
        deserializer.deserialize_struct(
            "Vrp", FIELDS, StructVisitor
        )
    }
}

impl Serialize for Vrp {
    fn serialize<S: serde::Serializer>(
        &self, serializer: S
    ) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let mut serializer = serializer.serialize_struct( "Vrp", 4)?;
        serializer.serialize_field(
            "prefix", &self.payload.prefix.prefix(),
        )?;
        serializer.serialize_field(
            "asn", &format!("{}", self.payload.asn),
        )?;
        serializer.serialize_field(
            "maxLength", &self.payload.prefix.max_len()
        )?;
        serializer.serialize_field(
            "ta", &self.ta
        )?;
        serializer.end()
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

    pub fn next_origin(&mut self) -> Option<PrefixOrigin> {
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
                            payload.prefix.unwrapped_max_len(),
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
                            payload.prefix.unwrapped_max_len(),
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
            assert_eq!(set.roas[0].ta, "ta");

            assert_eq!(set.roas[1].payload.asn, 4200000000.into());
            assert_eq!(
                set.roas[1].payload.prefix.addr(),
                IpAddr::from_str("2001:DB8::").unwrap()
            );
            assert_eq!(set.roas[1].payload.prefix.prefix_len(), 32);
            assert_eq!(set.roas[1].payload.prefix.max_len(), Some(32));
            assert_eq!(set.roas[1].ta, "ta");
        }

        check_set(serde_json::from_slice::<Set>(
            include_bytes!("../../test-data/vrps.json")
        ).unwrap());
        check_set(serde_json::from_slice::<Set>(
            include_bytes!("../../test-data/vrps-metadata.json")
        ).unwrap());
        check_set(serde_json::from_slice::<Set>(
            include_bytes!("../../test-data/vrps-metadata.rpki-client.json")
        ).unwrap());
    }
}

