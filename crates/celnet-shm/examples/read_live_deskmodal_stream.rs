//! Example reading live SBE OptionQuotes from the DeskModal CelNet pricing service ring buffer.

use celnet_shm::ShmConsumer;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

fn main() {
    let path = Path::new("/tmp/deskmodal-celnet-shm-quotes.ring");
    if !path.exists() {
        eprintln!("Ring file {:?} does not exist. Is the DeskModal pricing service running?", path);
        return;
    }

    println!("Attaching to live DeskModal CelNet SHM ring at {:?}...", path);
    let mut consumer = ShmConsumer::open_replay(path).expect("failed to open SHM consumer");

    println!("Connected! Ring capacity: {} slots, slot size: {} bytes", consumer.capacity(), consumer.slot_size());
    println!("Reading live SBE stream in-place via zero-copy view...\n");

    let mut count = 0;
    for _ in 0..20 {
        let result = consumer.try_recv_sbe_quote_view(|fw| {
            (
                fw.quote_id(),
                fw.bid_price(),
                fw.ask_price(),
                fw.resolved_strike(),
                fw.greeks().delta_spot,
                fw.greeks().gamma,
                fw.greeks().vega,
            )
        });

        match result {
            Ok((qid, bid, ask, strike, delta, gamma, vega)) => {
                count += 1;
                println!(
                    "  [Quote #{:05}] QID: {:<8} | Strike: {:.4} | Bid: {:.5} | Ask: {:.5} | Delta: {:+.4} | Gamma: {:+.4} | Vega: {:+.4}",
                    count, qid, strike, bid, ask, delta, gamma, vega
                );
            }
            Err(_) => {
                sleep(Duration::from_millis(50));
            }
        }
    }

    println!("\nSuccessfully received and validated {} live quotes from DeskModal SHM ring buffer!", count);
    println!("Consumer stats: received = {}, skipped = {}, cursor = {}", consumer.received(), consumer.skipped(), consumer.cursor());
}
