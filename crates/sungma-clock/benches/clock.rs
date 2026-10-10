//! The cost of reading each clock, alone and contended, and of a wait that
//! has nothing to wait for. Run with `cargo bench -p sungma-clock`.

use std::{
    net::{SocketAddr, UdpSocket},
    sync::OnceLock,
    thread,
};

use sungma::revision::Revision;
use sungma_clock::{Clock, ClockFault, Reading};
use sungma_clock::{ClockStatus, DevClock, ManualClock, NtpClock, Servers};

fn main() {
    // Sync before timing starts, so no bench measures the network.
    ntp();
    divan::main();
}

/// A server in step with the system clock that answers every request.
fn server() -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    thread::spawn(move || {
        loop {
            let mut request = [0; 48];
            let (_, client) = socket.recv_from(&mut request).unwrap();
            let mut reply = [0; 48];
            reply[0] = 0x24;
            reply[1] = 2;
            reply[8..12].copy_from_slice(&655u32.to_be_bytes());
            for at in [24, 32, 40] {
                reply[at..at + 8].copy_from_slice(&request[40..48]);
            }
            socket.send_to(&reply, client).unwrap();
        }
    });
    address
}

fn ntp() -> &'static NtpClock {
    static CLOCK: OnceLock<NtpClock> = OnceLock::new();
    CLOCK.get_or_init(|| NtpClock::sync(Servers::new((0..3).map(|_| server()))).unwrap())
}

fn passed() -> Reading {
    Reading {
        settled: Revision(0),
        revision: Revision(0),
    }
}

#[divan::bench(threads = [1, 2, 8, 32])]
fn ntp_now() -> Result<Reading, ClockFault> {
    ntp().now()
}

#[divan::bench(threads = [1, 2, 8, 32])]
fn dev_now() -> Result<Reading, ClockFault> {
    static CLOCK: OnceLock<DevClock> = OnceLock::new();
    CLOCK.get_or_init(DevClock::default).now()
}

#[divan::bench]
fn manual_now() -> Result<Reading, ClockFault> {
    static CLOCK: OnceLock<ManualClock> = OnceLock::new();
    CLOCK.get_or_init(|| ManualClock::new(passed())).now()
}

#[cfg(target_os = "linux")]
#[divan::bench(threads = [1, 2, 8, 32])]
fn linux_now() -> Option<Result<Reading, ClockFault>> {
    static CLOCK: OnceLock<Option<sungma_clock::LinuxClock>> = OnceLock::new();
    let clock = CLOCK
        .get_or_init(sungma_clock::LinuxClock::detect)
        .as_ref()?;
    Some(clock.now())
}

#[divan::bench]
fn ntp_status() -> ClockStatus {
    ntp().status()
}

#[divan::bench]
fn ntp_wait_on_a_passed_stamp(bencher: divan::Bencher) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let clock = ntp();
    bencher.bench_local(|| runtime.block_on(clock.wait(passed())));
}
