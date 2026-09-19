use nexus_model::{AppError, ErrorCode};

pub const MAX_REMOTE_PATH_BYTES: usize = 4096;

pub fn validate_remote_path(path: &str) -> Result<(), AppError> {
    if path.is_empty() || path.len() > MAX_REMOTE_PATH_BYTES || path.contains('\0') {
        return Err(policy(
            "The remote path is empty, contains NUL, or exceeds 4096 bytes.",
        ));
    }
    Ok(())
}

pub fn validate_child_name(name: &str) -> Result<(), AppError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\0')
        || name.len() > 255
    {
        return Err(policy("The server returned an unsafe child entry name."));
    }
    Ok(())
}

pub fn join_remote(parent: &str, child: &str) -> Result<String, AppError> {
    validate_remote_path(parent)?;
    validate_child_name(child)?;
    let joined = if parent == "/" {
        format!("/{child}")
    } else {
        format!("{}/{child}", parent.trim_end_matches('/'))
    };
    validate_remote_path(&joined)?;
    Ok(joined)
}

pub fn parent_remote(path: &str) -> Result<String, AppError> {
    validate_remote_path(path)?;
    if path == "/" {
        return Ok("/".into());
    }
    let trimmed = path.trim_end_matches('/');
    Ok(trimmed
        .rsplit_once('/')
        .map_or(
            ".",
            |(parent, _)| if parent.is_empty() { "/" } else { parent },
        )
        .into())
}

pub fn display_name(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character.is_control() || is_bidi_control(character) {
            use std::fmt::Write;
            let _ = write!(output, "\\u{{{:04X}}}", character as u32);
        } else {
            output.push(character);
        }
    }
    output
}

pub fn validate_windows_file_name(name: &str) -> Result<(), AppError> {
    validate_child_name(name)?;
    if name.ends_with(['.', ' '])
        || name.contains(['\\', ':', '*', '?', '"', '<', '>', '|'])
        || name.chars().any(char::is_control)
    {
        return Err(policy(
            "The remote name is not a valid Windows file name. Rename it explicitly before download.",
        ));
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end_matches(['.', ' ']);
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
    {
        return Err(policy(
            "The remote name is reserved by Windows. Rename it explicitly before download.",
        ));
    }
    Ok(())
}

fn is_bidi_control(c: char) -> bool {
    matches!(c as u32, 0x061C | 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069)
}

fn policy(message: &'static str) -> AppError {
    AppError::new(ErrorCode::FilePolicy, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_server_traversal_and_windows_aliases() {
        for name in ["", ".", "..", "a/b", "a\0b"] {
            assert!(validate_child_name(name).is_err());
        }
        for name in ["CON", "nul.txt", "name.", "a:b", "dir\\file"] {
            assert!(validate_windows_file_name(name).is_err());
        }
        assert_eq!(
            join_remote("/home/test", "кирилица ' -$.txt").unwrap(),
            "/home/test/кирилица ' -$.txt"
        );
    }

    #[test]
    fn escapes_controls_only_for_display() {
        assert_eq!(display_name("a\n\u{202e}b"), "a\\u{000A}\\u{202E}b");
    }
}
