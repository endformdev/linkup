use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

const LINKUP_DIR: &str = ".linkup";
const FINGERPRINT_FILE: &str = "fingerprint";

/// Generates a machine fingerprint based on hostname and MAC addresses.
/// The fingerprint format is: `{hostname_slug}-{hash_prefix}`
/// where hash_prefix is the first 6 characters of the SHA256 hash of sorted MAC addresses.
pub fn get_or_create_fingerprint() -> String {
    if let Ok(fingerprint) = read_stored_fingerprint() {
        return fingerprint;
    }

    let fingerprint = generate_fingerprint();
    store_fingerprint(&fingerprint);
    fingerprint
}

fn read_stored_fingerprint() -> Result<String, std::io::Error> {
    let fingerprint_path = get_fingerprint_path()?;
    fs::read_to_string(fingerprint_path)
}

fn store_fingerprint(fingerprint: &str) {
    if let Ok(path) = get_fingerprint_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(path, fingerprint);
    }
}

fn get_fingerprint_path() -> Result<PathBuf, std::io::Error> {
    let home_dir = dirs::home_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not determine home directory",
        )
    })?;
    Ok(home_dir.join(LINKUP_DIR).join(FINGERPRINT_FILE))
}

fn generate_fingerprint() -> String {
    let hostname = get_hostname();
    let hostname_slug = hostname_to_slug(&hostname);
    let mac_addresses = get_mac_addresses();
    let hash_prefix = generate_hash_prefix(&mac_addresses);

    format!("{}-{}", hostname_slug, hash_prefix)
}

fn get_hostname() -> String {
    std::env::var("HOSTNAME")
        .unwrap_or_else(|_| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .unwrap_or_else(|| "unknown".to_string())
        })
        .to_lowercase()
}

fn hostname_to_slug(hostname: &str) -> String {
    hostname
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' => c,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn get_mac_addresses() -> Vec<String> {
    if let Ok(output) = std::process::Command::new("ifconfig").arg("-a").output() {
        if output.status.success() {
            if let Ok(text) = String::from_utf8(output.stdout) {
                return parse_ifconfig_output(&text);
            }
        }
    }

    vec![format!("random-{}", rand::random::<u64>())]
}

fn parse_ifconfig_output(output: &str) -> Vec<String> {
    let mut macs = Vec::new();

    for line in output.lines() {
        if line.contains("ether") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            for part in parts {
                if part.contains(':') && part.len() == 17 {
                    let mac = part.to_lowercase();
                    if mac != "00:00:00:00:00:00" {
                        macs.push(mac);
                    }
                }
            }
        }
    }

    macs.sort();
    macs
}

fn generate_hash_prefix(mac_addresses: &[String]) -> String {
    let macs_string: String = mac_addresses.join("|");
    let mut hasher = Sha256::new();
    hasher.update(macs_string.as_bytes());
    let hash = hasher.finalize();
    hex::encode(&hash[..3])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hostname_to_slug() {
        assert_eq!(hostname_to_slug("MyHostName"), "myhostname");
        assert_eq!(hostname_to_slug("my-hostname"), "my-hostname");
        assert_eq!(hostname_to_slug("my_hostname"), "my-hostname");
        assert_eq!(hostname_to_slug("My Hostname"), "my-hostname");
    }

    #[test]
    fn test_generate_hash_prefix() {
        let macs = vec!["00:11:22:33:44:55".to_string()];
        let hash = generate_hash_prefix(&macs);
        assert_eq!(hash.len(), 6);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_fingerprint_format() {
        let fingerprint = generate_fingerprint();
        let parts: Vec<&str> = fingerprint.split('-').collect();
        assert!(
            parts.len() >= 2,
            "Fingerprint should have at least hostname and hash"
        );

        let last_part = parts.last().unwrap();
        assert_eq!(last_part.len(), 6);
        assert!(last_part.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
