#![no_main]

use std::collections::HashSet;
use libfuzzer_sys::fuzz_target;
use rpki::rtr::Payload;
use rtrtr::payload::{SetBuilder, PackBuilder};

fuzz_target!(|data: (Vec<Vec<Payload>>, HashSet<Payload>)| {
    // Make a set from the first item in data.
    let mut builder = SetBuilder::empty();
    let mut payload = HashSet::<Payload>::default();
    for item in data.0 {
        let mut pack = PackBuilder::empty();
        for item in item {
            if pack.insert(item.clone()).is_ok() {
                payload.insert(item.clone());
            }
        }
        builder.insert_pack(pack.finalize())
    }
    let set = builder.finalize();
 
    // Now filter everything in data.1 out of both set and payload.
    let filtered_set = set.filter(|item| data.1.contains(item));
    let mut filtered_payload =
        payload.intersection(&data.1).cloned().collect::<Vec<_>>();
    filtered_payload.sort();
    
    let eq = filtered_set.iter().eq(filtered_payload.iter());
    if !eq {
        eprintln!("filtered_set: {:#?}", filtered_set);
        eprintln!("filtered_payload: {:#?}", filtered_payload);
    }
    assert!(eq);
});
