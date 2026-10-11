//! Runs NTS reply verification on arbitrary bytes, and on sealed
//! plaintexts: it must refuse or accept every reply without panicking, and
//! keep at most eight cookies.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| sungma_clock::fuzz_verify(data));
