//! Runs the NTP extension field parser on arbitrary bytes: it must refuse
//! or accept every input without panicking, and its fields must account
//! for every byte.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| sungma_clock::fuzz_fields(data));
