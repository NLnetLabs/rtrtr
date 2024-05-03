#![no_main]
#![feature(is_sorted)]

use std::collections::HashSet;
use libfuzzer_sys::fuzz_target;
use rpki::rtr::Payload;
use rtrtr::payload::{Set, SetBuilder, PackBuilder};

fn make_set(data: &Vec<Vec<Payload>>) -> (Set, HashSet<Payload>) {
    let mut builder = SetBuilder::empty();
    let mut payload = HashSet::<Payload>::default();
    for item in data {
        let mut pack = PackBuilder::empty();
        for item in item {
            if pack.insert(item.clone()).is_ok() {
                payload.insert(item.clone());
            }
        }
        builder.insert_pack(pack.finalize())
    }
    let set = builder.finalize();
    (set, payload)
}

fuzz_target!(|data: (Vec<Vec<Payload>>, Vec<Vec<Payload>>)| {
    let (left_set, left_hash) = make_set(&data.0);
    let (right_set, right_hash) = make_set(&data.1);

    // Check that the sets are in correct order.
    // Iterator::is_sorted is unstable, but we have to use nightly anyway.
    assert!(left_set.iter().is_sorted());
    assert!(right_set.iter().is_sorted());

    eprintln!("Sets are fine.");


    let merged_set = left_set.merge(&right_set);
    let mut merged_vec = left_hash.union(
        &right_hash
    ).cloned().collect::<Vec<_>>();
    merged_vec.sort();
    let eq = merged_set.iter().eq(merged_vec.iter());
    if !eq {
        eprintln!("Left set: {:#?}", data.0);
        eprintln!("Right set: {:#?}", data.1);
        eprintln!("Result: {:#?}", merged_set);
        eprintln!("Result iterated: {:#?}",
            merged_set.iter().collect::<Vec<_>>()
        );
        eprintln!("Result set: {:#?}", merged_vec);
    }
    assert!(eq);
});
