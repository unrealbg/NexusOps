use nexus_model::{AppError, ErrorCode, MAX_REMOTE_EDITOR_BYTES, TextNewline};

pub fn decode_text(bytes: &[u8]) -> Result<(String, TextNewline, bool), AppError> {
    if bytes.len() > MAX_REMOTE_EDITOR_BYTES {
        return Err(policy(
            "The remote text file exceeds the 1 MiB editor limit.",
        ));
    }
    let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
    let body = if bom { &bytes[3..] } else { bytes };
    let original = std::str::from_utf8(body)
        .map_err(|_| policy("The remote file is not valid UTF-8 text."))?;
    if original.contains('\0') {
        return Err(policy("NUL bytes are not supported by the text editor."));
    }
    let has_crlf = original.contains("\r\n");
    let without_crlf = original.replace("\r\n", "");
    if without_crlf.contains('\r') || (has_crlf && without_crlf.contains('\n')) {
        return Err(policy(
            "Mixed or unsupported newline conventions cannot be edited safely.",
        ));
    }
    let newline = if has_crlf {
        TextNewline::CrLf
    } else {
        TextNewline::Lf
    };
    let text = if has_crlf {
        original.replace("\r\n", "\n")
    } else {
        original.to_owned()
    };
    Ok((text, newline, bom))
}

pub fn encode_text(text: &str, newline: TextNewline, bom: bool) -> Result<Vec<u8>, AppError> {
    if text.len() + if bom { 3 } else { 0 } > MAX_REMOTE_EDITOR_BYTES {
        return Err(policy("The edited text exceeds the 1 MiB editor limit."));
    }
    if text.contains('\0') || text.contains('\r') {
        return Err(policy(
            "The editor accepts LF-normalized UTF-8 text without NUL bytes.",
        ));
    }
    let body = match newline {
        TextNewline::Lf => text.to_owned(),
        TextNewline::CrLf => text.replace('\n', "\r\n"),
    };
    let size = body.len() + if bom { 3 } else { 0 };
    if size > MAX_REMOTE_EDITOR_BYTES {
        return Err(policy("The edited text exceeds the 1 MiB editor limit."));
    }
    let mut bytes = Vec::with_capacity(size);
    if bom {
        bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
    }
    bytes.extend_from_slice(body.as_bytes());
    Ok(bytes)
}

fn policy(message: &'static str) -> AppError {
    AppError::new(ErrorCode::FilePolicy, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_text_roundtrips() {
        for data in [
            b"hello\n".as_slice(),
            b"hello\r\n".as_slice(),
            b"hello".as_slice(),
            b"\xef\xbb\xbfhello\r\n".as_slice(),
        ] {
            let (text, newline, bom) = decode_text(data).unwrap();
            assert_eq!(encode_text(&text, newline, bom).unwrap(), data);
        }
    }
    #[test]
    fn rejects_invalid_and_ambiguous() {
        for data in [
            b"a\0b".as_slice(),
            b"a\r\nb\n".as_slice(),
            b"a\r".as_slice(),
            b"\xff".as_slice(),
        ] {
            assert!(decode_text(data).is_err());
        }
    }
    #[test]
    fn cap_is_exact() {
        assert!(decode_text(&vec![b'a'; MAX_REMOTE_EDITOR_BYTES]).is_ok());
        assert!(decode_text(&vec![b'a'; MAX_REMOTE_EDITOR_BYTES + 1]).is_err());
    }
}
