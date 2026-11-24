use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const MODEL_FILE: &str = "moonshine-streaming-tiny-Q8_0.gguf";
const MODEL_URL: &str = "https://huggingface.co/handy-computer/moonshine-streaming-tiny-gguf/resolve/4125c28c22a76494162094ab3566387cd92fb2f5/moonshine-streaming-tiny-Q8_0.gguf";
const MODEL_SHA256: &str = "930e4622ad3a24158b91406c30c977fa6a26b34cb32d6ac3e57cfb23383a869e";

const FAILED: &str = "Could not download the speech model.";

fn sha256(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn install(part: &Path, dest: &Path, expected: &str) -> Result<PathBuf, &'static str> {
    if !sha256(part).is_ok_and(|hash| hash == expected) {
        let _ = fs::remove_file(part);
        return Err("The downloaded speech model failed verification.");
    }
    if fs::rename(part, dest).is_err() {
        let _ = fs::remove_file(part);
        return Err("Could not save the speech model.");
    }
    Ok(dest.to_path_buf())
}

async fn fetch(
    http: &reqwest::Client,
    part: &Path,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<(), &'static str> {
    let mut response = http
        .get(MODEL_URL)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|_| FAILED)?;
    let total = response.content_length().unwrap_or(0);
    let mut file = fs::File::create(part).map_err(|_| FAILED)?;
    let mut done = 0;
    while let Some(chunk) = response.chunk().await.map_err(|_| FAILED)? {
        file.write_all(&chunk).map_err(|_| FAILED)?;
        done += chunk.len() as u64;
        on_progress(done, total);
    }
    Ok(())
}

pub async fn ensure_model(
    http: &reqwest::Client,
    dir: &Path,
    on_progress: impl FnMut(u64, u64),
) -> Result<PathBuf, &'static str> {
    let dest = dir.join(MODEL_FILE);
    if sha256(&dest).is_ok_and(|hash| hash == MODEL_SHA256) {
        return Ok(dest);
    }
    fs::create_dir_all(dir).map_err(|_| FAILED)?;
    let part = dir.join(format!("{MODEL_FILE}.part"));
    if let Err(e) = fetch(http, &part, on_progress).await {
        let _ = fs::remove_file(&part);
        return Err(e);
    }
    install(&part, &dest, MODEL_SHA256)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("argos-speech-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("model.part"), "hello").unwrap();
        dir
    }

    #[test]
    fn hash_mismatch_rejects_and_leaves_no_file() {
        let dir = scratch("mismatch");
        let wrong = "0".repeat(64);
        assert!(install(&dir.join("model.part"), &dir.join("model"), &wrong).is_err());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn verified_download_is_renamed_into_place() {
        let dir = scratch("match");
        let dest = install(&dir.join("model.part"), &dir.join("model"), HELLO_SHA256).unwrap();
        assert_eq!(fs::read(dest).unwrap(), b"hello");
        assert!(!dir.join("model.part").exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
