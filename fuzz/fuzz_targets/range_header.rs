//! HTTP Range headers: any reply must stay inside the representation.
#![no_main]
use abshare_core::range::{RangeReply, parse};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|i: (&str, u64)| {
    let (h, len) = i;
    if let RangeReply::Partial { start, end } = parse(Some(h), len) {
        assert!(start <= end && end < len);
    }
});
