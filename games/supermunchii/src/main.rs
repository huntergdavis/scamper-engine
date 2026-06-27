//! `scamp` — the Super Munchii binary. A thin forwarder: all logic lives in the
//! `supermunchii` library crate so the arcade launcher can reuse it. This just hands
//! argv to `run_cli` for the standalone subcommand workflow.

fn main() {
    supermunchii::run_cli(std::env::args().collect());
}
