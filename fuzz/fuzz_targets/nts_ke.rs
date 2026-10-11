//! Runs the NTS key exchange response parser on arbitrary bytes: it must
//! refuse or accept every response without panicking, and keep at most
//! eight cookies.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| sungma_clock::fuzz_ke_response(data));
