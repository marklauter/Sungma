//! Runs the NTP reply parser on arbitrary bytes: it must refuse or accept
//! every reply without panicking or overflowing.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| sungma_clock::fuzz_answer(data));
