// src/bin/sign_license.rs
//
// License signing tool for CheckpointPro. Run from the project root:
//     cargo run --bin sign_license
//
// Prompts for VERSION, LICENSEE, EXPIRY, signs `version|licensee|expiry` with
// the Ed25519 private key, and prints the copy-pasteable license block plus
// appends a record to issued_licenses.log.
//
// The canonical payload format MUST match RegistrationInfo::validate exactly:
//     format!("{version}|{licensee}|{expiry}")
// If you ever change one, change both, or every issued license fails to verify.
//
// Add to Cargo.toml so this binary picks up the deps:
//   (ed25519-dalek and base64 are already dependencies of the main crate)

use std::fs::OpenOptions;
use std::io::{self, Write};

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Local;
use ed25519_dalek::{Signer, SigningKey};

fn prompt(label: &str) -> String {
    print!("{label}: ");
    io::stdout().flush().unwrap();
    let mut s = String::new();
    io::stdin().read_line(&mut s).unwrap();
    s.trim().to_string()
}

fn main() {
    let key_path = "assets/license_private.key";

    let key_bytes = std::fs::read(key_path).unwrap_or_else(|e| {
        eprintln!("Failed to read private key at {key_path}: {e}");
        std::process::exit(1);
    });

    // Expect 32 raw bytes
    let key_array: [u8; 32] = key_bytes.as_slice().try_into().unwrap_or_else(|_| {
        eprintln!(
            "Private key must be exactly 32 raw bytes (got {}). \
             If it is base64 or has a trailing newline, fix the file.",
            key_bytes.len()
        );
        std::process::exit(1);
    });
    let signing_key = SigningKey::from_bytes(&key_array);

    // --- Gather the fields ---------------------------------------------------
    println!("--- CheckpointPro license signer ---");
    let version = prompt("VERSION (e.g. 1.0)");
    let licensee = prompt("LICENSEE (institution name)");
    let expiry = prompt("EXPIRY (YYYY-MM-DD)");

    // Light validation so you don't sign a malformed date and discover it later.
    if chrono::NaiveDate::parse_from_str(&expiry, "%Y-%m-%d").is_err() {
        eprintln!("EXPIRY '{expiry}' is not a valid YYYY-MM-DD date. Aborting.");
        std::process::exit(1);
    }
    if version.is_empty() || licensee.is_empty() {
        eprintln!("VERSION and LICENSEE must not be empty. Aborting.");
        std::process::exit(1);
    }

    // --- Sign ----------------------------------------------------------------
    // MUST match RegistrationInfo::validate's payload construction.
    let payload = format!("{version}|{licensee}|{expiry}");
    let signature = signing_key.sign(payload.as_bytes());
    let prod_key = STANDARD.encode(signature.to_bytes());

    // --- Emit the block ------------------------------------------------------
    let block = format!(
        "=============START OF LICENSE=============\n\
         VERSION: {version}\n\
         LICENSEE: {licensee}\n\
         EXPIRY: {expiry}\n\
         PROD_KEY: {prod_key}\n\
         ==============END OF LICENSE=============="
    );

    println!("\n{block}\n");

    // --- Log it so you can answer 'resend our key' later ---------------------
    let log_line = format!(
        "{}\tVERSION={}\tLICENSEE={}\tEXPIRY={}\tPROD_KEY={}\n",
        Local::now().format("%Y-%m-%d %H:%M:%S"),
        version,
        licensee,
        expiry,
        prod_key
    );
    match OpenOptions::new()
        .create(true)
        .append(true)
        .open("issued_licenses.log")
    {
        Ok(mut f) => {
            if let Err(e) = f.write_all(log_line.as_bytes()) {
                eprintln!("(warning) could not write to issued_licenses.log: {e}");
            }
        }
        Err(e) => eprintln!("(warning) could not open issued_licenses.log: {e}"),
    }
}
