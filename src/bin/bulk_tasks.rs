//! Bulk task creation binary. See `claude_reliability::tasks::bulk` for docs.

use claude_reliability::tasks::bulk;
use std::io::{self, Read};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] == "--help" || args[1] == "-h" {
        bulk::print_usage();
        std::process::exit(i32::from(args.len() < 2));
    }
    if let Err(e) = run(&args[1]) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run(cmd: &str) -> Result<(), Box<dyn std::error::Error>> {
    let store = bulk::open_default_store()?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let output = match cmd {
        "create" => serde_json::to_string_pretty(&bulk::create_from_json(&store, &input)?)?,
        "add-deps" => serde_json::to_string_pretty(&bulk::add_deps_from_json(&store, &input)?)?,
        "list" => serde_json::to_string_pretty(&bulk::list_from_json(&store, &input)?)?,
        "search" => serde_json::to_string_pretty(&bulk::search_from_json(&store, &input)?)?,
        other => {
            eprintln!("Unknown command: {other}");
            bulk::print_usage();
            std::process::exit(1);
        }
    };
    println!("{output}");
    Ok(())
}
