use nexus_model::{AppError, ErrorCode};
use std::collections::BTreeMap;

fn invalid() -> AppError {
    // Never include remote-controlled data in diagnostics.
    AppError::new(
        ErrorCode::Discovery,
        "The host returned an unsupported discovery value.",
    )
}

pub(crate) fn scalar(output: &str) -> Result<String, AppError> {
    let value = output.trim();
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(value.to_string())
}

pub(crate) fn first_number(output: &str) -> Result<f64, AppError> {
    let number = output
        .split_whitespace()
        .next()
        .ok_or_else(invalid)?
        .parse::<f64>()
        .map_err(|_| invalid())?;
    if !number.is_finite() || number < 0.0 {
        return Err(invalid());
    }
    Ok(number)
}

/// Parse os-release assignments without interpreting shell syntax or expansion.
pub(crate) fn os_release(output: &str) -> Result<(String, Option<String>), AppError> {
    let mut values = BTreeMap::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if ["NAME", "ID", "VERSION_ID", "VERSION"].contains(&key) {
            values.insert(key, assignment_value(value.trim())?);
        }
    }
    let name = values
        .get("NAME")
        .or_else(|| values.get("ID"))
        .ok_or_else(invalid)?
        .clone();
    if name.is_empty() {
        return Err(invalid());
    }
    let version = values
        .get("VERSION_ID")
        .or_else(|| values.get("VERSION"))
        .filter(|value| !value.is_empty())
        .cloned();
    Ok((name, version))
}

fn assignment_value(input: &str) -> Result<String, AppError> {
    if input.is_empty() {
        return Ok(String::new());
    }
    let quote = input.chars().next().ok_or_else(invalid)?;
    let value = if quote == '"' || quote == '\'' {
        if input.len() < 2 || !input.ends_with(quote) {
            return Err(invalid());
        }
        let inner = &input[1..input.len() - 1];
        if quote == '\'' {
            if inner.contains('\'') {
                return Err(invalid());
            }
            inner.to_owned()
        } else {
            let mut result = String::new();
            let mut chars = inner.chars().peekable();
            while let Some(character) = chars.next() {
                if character == '"' {
                    return Err(invalid());
                }
                if character == '\\' {
                    let next = chars.peek().copied().ok_or_else(invalid)?;
                    if matches!(next, '$' | '`' | '"' | '\\') {
                        result.push(chars.next().ok_or_else(invalid)?);
                        continue;
                    }
                }
                result.push(character);
            }
            result
        }
    } else {
        if input.chars().any(char::is_whitespace) || input.contains(['\'', '"']) {
            return Err(invalid());
        }
        input.to_string()
    };
    if value.is_empty() {
        Ok(value)
    } else {
        scalar(&value)
    }
}

fn kib_bytes(value: &str) -> Result<u64, AppError> {
    value
        .parse::<u64>()
        .map_err(|_| invalid())?
        .checked_mul(1024)
        .ok_or_else(invalid)
}

pub(crate) fn memory(output: &str) -> Result<(u64, u64), AppError> {
    let mut values = BTreeMap::new();
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if ![
            "MemTotal",
            "MemAvailable",
            "MemFree",
            "Buffers",
            "Cached",
            "SReclaimable",
            "Shmem",
        ]
        .contains(&key)
        {
            continue;
        }
        let mut parts = value.split_whitespace();
        let number = parts.next().ok_or_else(invalid)?;
        if parts.next() != Some("kB") || parts.next().is_some() {
            return Err(invalid());
        }
        if values.insert(key, kib_bytes(number)?).is_some() {
            return Err(invalid());
        }
    }
    let total = *values.get("MemTotal").ok_or_else(invalid)?;
    if total == 0 {
        return Err(invalid());
    }
    let available = if let Some(available) = values.get("MemAvailable") {
        *available
    } else {
        // Older kernels lack MemAvailable. This is a conventional approximation.
        let mut estimate = *values.get("MemFree").ok_or_else(invalid)?;
        for key in ["Buffers", "Cached", "SReclaimable"] {
            estimate = estimate
                .checked_add(*values.get(key).unwrap_or(&0))
                .ok_or_else(invalid)?;
        }
        estimate
            .saturating_sub(*values.get("Shmem").unwrap_or(&0))
            .min(total)
    };
    let used = total.checked_sub(available).ok_or_else(invalid)?;
    Ok((total, used))
}

pub(crate) fn root_filesystem(output: &str) -> Result<(u64, u64), AppError> {
    // POSIX df -P keeps each filesystem on one line. Parse from the mount point
    // backwards so spaces in the filesystem identifier do not shift columns.
    let row = output
        .lines()
        .find(|line| line.split_whitespace().next_back() == Some("/"))
        .ok_or_else(invalid)?;
    let mut parts = row.split_whitespace().rev();
    parts.next(); // root mount point
    let capacity = parts.next().ok_or_else(invalid)?;
    if !capacity.ends_with('%') {
        return Err(invalid());
    }
    let available = parts.next().ok_or_else(invalid)?;
    available.parse::<i128>().map_err(|_| invalid())?; // May be negative on a full filesystem.
    let used = kib_bytes(parts.next().ok_or_else(invalid)?)?;
    let total = kib_bytes(parts.next().ok_or_else(invalid)?)?;
    if parts.next().is_none() || total == 0 || used > total {
        return Err(invalid());
    }
    Ok((total, used))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_os_release_as_data_with_quotes_and_equals() {
        let (name, version) = os_release("# distro\nNAME=\"Example \\\"Linux\\\" = Stable\"\nVERSION_ID='24.04'\nUNUSED=$(danger)\n").expect("os release");
        assert_eq!(name, "Example \"Linux\" = Stable");
        assert_eq!(version.as_deref(), Some("24.04"));
        assert_eq!(
            os_release("NAME=\"$(touch /tmp/never)\"\n")
                .expect("literal shell syntax")
                .0,
            "$(touch /tmp/never)"
        );
        assert_eq!(
            os_release("ID=alpine\nVERSION_ID=\"\"\n").expect("minimal OS"),
            ("alpine".into(), None)
        );
    }

    #[test]
    fn rejects_malformed_text_and_non_finite_metrics() {
        for value in ["", "-1", "NaN", "inf", "1e999"] {
            assert!(first_number(value).is_err());
        }
        assert_eq!(first_number("123.45 900.00\n").expect("uptime"), 123.45);
        assert!(scalar("host\nsecond line").is_err());
        assert!(scalar("host\u{1b}[31m").is_err());
        assert!(os_release("NAME=\"unterminated\n").is_err());
        assert!(os_release("NAME=bad name\n").is_err());
    }

    #[test]
    fn uses_available_memory_and_supports_old_kernels() {
        assert_eq!(
            memory("MemTotal: 1000 kB\nMemAvailable: 250 kB\nHugePages_Total: 0\n")
                .expect("memory"),
            (1024000, 768000)
        );
        assert_eq!(memory("MemTotal: 1000 kB\nMemFree: 100 kB\nBuffers: 50 kB\nCached: 200 kB\nSReclaimable: 25 kB\nShmem: 10 kB\n").expect("old kernel"), (1024000, 650240));
    }

    #[test]
    fn rejects_memory_overflow_units_duplicates_and_inconsistent_values() {
        for value in [
            "MemTotal: 18446744073709551615 kB\nMemAvailable: 1 kB",
            "MemTotal: 100 MB\nMemAvailable: 1 MB",
            "MemTotal: 100 kB\nMemAvailable: 101 kB",
            "MemTotal: 100 kB\nMemTotal: 100 kB\nMemAvailable: 1 kB",
        ] {
            assert!(memory(value).is_err());
        }
    }

    #[test]
    fn reads_posix_df_and_checks_bounds() {
        assert_eq!(root_filesystem("Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/root 1000 400 550 43% /\n").expect("disk"), (1024000, 409600));
        assert_eq!(
            root_filesystem("device with spaces 1000 400 550 43% /\n").expect("device spaces"),
            (1024000, 409600)
        );
        assert!(root_filesystem("dev 100 101 0 101% /\n").is_err());
        assert!(root_filesystem("dev 18446744073709551615 1 0 1% /\n").is_err());
        assert!(root_filesystem("dev 1000 400 550 43% /home\n").is_err());
    }
}
