//! Manual verification for `reveal_in_file_manager` (the Phase 11
//! cross-platform seam).
//!
//! ```bash
//! cargo run -p platform --example reveal -- <file>
//! ```
//!
//! Opens the platform file manager focused on `<file>`: Finder with the
//! file selected on macOS, Explorer with it selected on Windows, the
//! containing folder on Linux.

use std::path::PathBuf;

fn main() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            eprintln!("usage: reveal <file>");
            std::process::exit(2);
        });
    match platform::reveal_in_file_manager(&path) {
        Ok(()) => println!("reveal requested for {}", path.display()),
        Err(error) => {
            eprintln!("reveal failed: {error}");
            std::process::exit(1);
        }
    }
}
