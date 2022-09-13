#![feature(async_iterator)]
#![feature(result_flattening)]

pub use listener::UtpListener;
pub use stream::UtpStream;
pub use socket::UtpSocket;

mod bit_iterator;
mod error;
mod listener;
mod packet;
mod socket;
mod stream;
mod time;
mod util;

#[cfg(test)]
mod tests {
    fn next_test_port() -> u16 {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT_OFFSET: AtomicUsize = AtomicUsize::new(0);
        const BASE_PORT: u16 = 10_000;
        BASE_PORT + NEXT_OFFSET.fetch_add(1, Ordering::Relaxed) as u16
    }

    pub fn next_test_ip4<'a>() -> (&'a str, u16) {
        ("127.0.0.1", next_test_port())
    }

    pub fn next_test_ip6<'a>() -> (&'a str, u16) {
        ("::1", next_test_port())
    }
}
