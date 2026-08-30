//! Prints the JSON Schema for `config.toml` on stdout. The checked-in
//! artifact at `crates/recoil-core/schema/config.schema.json` is produced
//! by this example and guarded against drift by a unit test.
//!
//! The workspace bans print macros in application code (tracing owns
//! diagnostics); a schema generator writing to stdout is the exception.

#![allow(clippy::print_stdout, clippy::print_stderr)]

fn main() {
  match recoil_core::config::Config::json_schema() {
    Ok(schema) => println!("{schema}"),
    Err(err) => {
      eprintln!("schema export failed: {err}");
      std::process::exit(1);
    }
  }
}
