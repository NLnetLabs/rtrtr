#![no_main]

use std::collections::HashSet;
use libfuzzer_sys::fuzz_target;
use rpki::rtr::Payload;
use rtrtr::payload::{SetBuilder, PackBuilder};

fuzz_target!(|data: Vec<Vec<Payload>> | {
    let mut builder = SetBuilder::empty();
    let mut payload = HashSet::<Payload>::default();
    for item in data {
        let mut pack = PackBuilder::empty();
        for item in item {
            if pack.insert(item.clone()).is_ok() {
                payload.insert(item);
            }
        }
        builder.insert_pack(pack.finalize())
    }
    let set = builder.finalize();
    for item in set.iter() {
        assert!(payload.remove(item));
    }
    assert!(payload.is_empty());
});
