//! The Zoomies binary — a thin forwarder. All logic lives in the `zoomies` library
//! so the arcade launcher can reuse it; this hands argv to `run_cli`.

fn main() {
    zoomies::run_cli(std::env::args().collect());
}
