use std::path::PathBuf;

use base64::{Engine, engine::general_purpose};
use chrono::{DateTime, Local, TimeZone};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use crate::{LICENSE_PUBLIC_KEY, err};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseSaveResult {
    MachineWide,
    UserOnly,
}

fn machine_wide_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    return PathBuf::from(std::env::var_os("ProgramData").unwrap_or_default())
        .join("CheckpointPro/license.txt");
    #[cfg(target_os = "macos")]
    return PathBuf::from("/Library/Application Support/CheckpointPro/license.txt");
    #[cfg(target_os = "linux")]
    return PathBuf::from("/etc/CheckpointPro/license.txt");
}

fn user_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    return PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
        .join("CheckpointPro/license.txt");
    #[cfg(target_os = "macos")]
    return PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support/CheckpointPro/license.txt");
    #[cfg(target_os = "linux")]
    return PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join(".config/CheckpointPro/license.txt");
}

fn try_write(path: &PathBuf, license: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, license)
}

/// Saves machine-wide if permitted, otherwise to the user's config dir.
pub fn save_license(license: String) -> Result<LicenseSaveResult, err::Reason> {
    if try_write(&machine_wide_path(), &license).is_ok() {
        return Ok(LicenseSaveResult::MachineWide);
    }
    let path = user_path();
    try_write(&path, &license).map_err(|e| err::Reason::Io(path, e))?;
    Ok(LicenseSaveResult::UserOnly)
}

pub fn load_license() -> Option<String> {
    std::fs::read_to_string(machine_wide_path())
        .or_else(|_| std::fs::read_to_string(user_path()))
        .ok()
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Registration {
    Unregistered,
    Expired,
    Registered,
}

#[derive(Debug, Clone)]
pub struct RegistrationInfo {
    pub version: String,
    pub licensee: String,
    pub expiry: DateTime<Local>,
    pub expiry_raw: String,
    pub product_key: String,
}

impl RegistrationInfo {
    /// Constructs product registration info from a license.
    /// Liscences follow the following format
    /// =============START OF LICENSE=============
    /// VERSION: 1.0
    /// LICENSEE: Wayside School
    /// EXPIRY: YYYY-MM-DD
    /// PROD_KEY: ABCDEFGHIJKLMNOPQRSTUVWXYZ012345
    /// ==============END OF LICENSE==============
    pub fn new(info: String) -> Result<RegistrationInfo, err::Reason> {
        let mut lines = info.lines();
        lines.next(); // discard "===START OF LICENSE==="

        let version = lines
            .next()
            .and_then(|l| l.strip_prefix("VERSION:"))
            .map(|v| v.trim().to_string())
            .ok_or(err::Reason::Other(
                "Failed to read version number".to_string(),
            ))?;

        let licensee = lines
            .next()
            .and_then(|l| l.strip_prefix("LICENSEE:"))
            .map(|v| v.trim().to_string())
            .ok_or(err::Reason::Other("Failed to read licensee".to_string()))?;

        let expiry_raw = lines
            .next()
            .and_then(|l| l.strip_prefix("EXPIRY:"))
            .map(|v| v.trim().to_string())
            .ok_or(err::Reason::Other("Failed to read expiry".to_string()))?;

        let expiry_date = chrono::NaiveDate::parse_from_str(&expiry_raw, "%Y-%m-%d")
            .map_err(|_| err::Reason::Other("Invalid expiry date format".to_string()))?;

        let expiry = expiry_date
            .and_hms_opt(23, 59, 59)
            .and_then(|naive| Local.from_local_datetime(&naive).single())
            .ok_or(err::Reason::Other(
                "Could not construct expiry datetime".to_string(),
            ))?;

        let product_key = lines
            .next()
            .and_then(|l| l.strip_prefix("PROD_KEY:"))
            .map(|v| v.trim().to_string())
            .ok_or(err::Reason::Other("Failed to read product key".to_string()))?;

        Ok(Self {
            version,
            licensee,
            expiry,
            expiry_raw,
            product_key,
        })
    }

    pub fn validate(&self) -> Registration {
        let payload = format!("{}|{}|{}", self.version, self.licensee, self.expiry_raw);

        let base64 = general_purpose::STANDARD;
        let signature = match base64.decode(self.product_key.clone()) {
            Ok(s) => s,
            Err(_) => return Registration::Unregistered,
        };

        let signature = match Signature::from_slice(&signature) {
            Ok(s) => s,
            Err(_) => return Registration::Unregistered,
        };

        let key = match VerifyingKey::from_bytes(LICENSE_PUBLIC_KEY) {
            Ok(k) => k,
            Err(_) => panic!("CheckpointPro encountered a problem during registration"),
        };

        match key.verify(payload.as_bytes(), &signature) {
            Ok(_) => {
                if self.expiry + chrono::Duration::days(1) < Local::now() {
                    Registration::Expired
                } else {
                    Registration::Registered
                }
            }
            Err(_) => Registration::Unregistered,
        }
    }
}
