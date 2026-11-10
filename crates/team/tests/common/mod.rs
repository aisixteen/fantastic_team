//! Helpers the binary-driving tests share: one test at a time, and a free port that no
//! other test in the process was handed.
#![allow(dead_code)]

pub mod boot;

use std::collections::BTreeSet;
use std::net::TcpListener;
use std::sync::{Mutex, MutexGuard};

/// Every test holds this for its whole body, so two tests never race for a
/// port, a child process or a stray listener.
static SERIAL: Mutex<()> = Mutex::new(());

pub fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ports already handed out in this test process; `free_port` never repeats one.
static ISSUED: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

pub fn free_port() -> u16 {
    let mut issued = ISSUED.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        if issued.insert(port) {
            return port;
        }
    }
}
