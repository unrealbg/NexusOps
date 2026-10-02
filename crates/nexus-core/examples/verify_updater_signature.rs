use base64::{Engine as _, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use std::{
    env,
    fs::{File, metadata, read},
    io::{self, Read},
    path::Path,
};

const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 4096;

fn bounded_text(path: &Path, label: &str) -> Result<String, String> {
    let data = read(path).map_err(|_| format!("{label} is unreadable"))?;
    if data.is_empty() || data.len() > MAX_TEXT_BYTES {
        return Err(format!("{label} size is invalid"));
    }
    let encoded = std::str::from_utf8(&data).map_err(|_| format!("{label} is not UTF-8"))?;
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| format!("{label} wrapper is not base64"))?;
    if decoded.len() > MAX_TEXT_BYTES {
        return Err(format!("{label} decoded size is invalid"));
    }
    String::from_utf8(decoded).map_err(|_| format!("{label} decoded text is not UTF-8"))
}

fn validate_simple(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 255
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err(format!("{label} is invalid"));
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 4 {
        return Err("expected <artifact> <signature> <public-key> <product-version>".to_owned());
    }
    let artifact_path = Path::new(&args[0]);
    let signature_path = Path::new(&args[1]);
    let public_key_path = Path::new(&args[2]);
    let version = args[3]
        .to_str()
        .ok_or_else(|| "product version is not UTF-8".to_owned())?;
    validate_simple(version, "product version")?;
    let artifact_name = artifact_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "artifact basename is invalid".to_owned())?;
    validate_simple(artifact_name, "artifact basename")?;

    let info = metadata(artifact_path).map_err(|_| "artifact is unreadable".to_owned())?;
    if !info.is_file() || info.len() == 0 || info.len() > MAX_ARTIFACT_BYTES {
        return Err("artifact size is invalid".to_owned());
    }
    let public_key = PublicKey::decode(&bounded_text(public_key_path, "public key")?)
        .map_err(|_| "public key decode failed".to_owned())?;
    let signature = Signature::decode(&bounded_text(signature_path, "signature")?)
        .map_err(|_| "signature decode failed".to_owned())?;
    let mut verifier = public_key
        .verify_stream(&signature)
        .map_err(|_| "signature verifier initialization failed".to_owned())?;
    let mut file = File::open(artifact_path).map_err(|_| "artifact is unreadable".to_owned())?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| "artifact read failed".to_owned())?;
        if count == 0 {
            break;
        }
        verifier.update(&buffer[..count]);
    }
    verifier
        .finalize()
        .map_err(|_| "cryptographic signature verification failed".to_owned())?;

    let expected_file = format!("file:{artifact_name}");
    let expected_version = format!("version:{version}");
    let fields: Vec<_> = signature.trusted_comment().split('\t').collect();
    if fields
        .iter()
        .filter(|field| **field == expected_file)
        .count()
        != 1
        || fields
            .iter()
            .filter(|field| **field == expected_version)
            .count()
            != 1
    {
        return Err("trusted comment lacks exact artifact/version binding".to_owned());
    }
    Ok(())
}

fn main() {
    if let Err(message) = run() {
        let _ = writeln_stderr(&message);
        std::process::exit(1);
    }
}

fn writeln_stderr(message: &str) -> io::Result<()> {
    use std::io::Write as _;
    writeln!(
        io::stderr().lock(),
        "Updater signature verification failed: {message}"
    )
}
