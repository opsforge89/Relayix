//! Full database backup envelope: Argon2id and authenticated, ordered AES-GCM frames.
use crate::*;
use argon2::{Algorithm, Argon2, Params, Version};
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom, Write},
};
const MAGIC: &[u8; 16] = b"RELAYIX-BACKUP\x01\x00";
const CHUNK: usize = 64 * 1024;
pub const SETTINGS: &[&str] = &[
    "SECRETS_KEY",
    "ADMIN_USERNAME",
    "ADMIN_PASSWORD",
    "ADMIN_TOKEN",
    "TELEGRAM_API_ID",
    "TELEGRAM_API_HASH",
    "COOKIE_SECURE",
    "ALLOW_INSECURE_HTTP",
    "TRUSTED_PROXIES",
    "OUTBOUND_ALLOWLIST",
    "WEBHOOK_RATE_LIMIT_PER_MINUTE",
    "DELIVERY_WORKERS",
    "DELIVERY_PROXY_CONCURRENCY",
    "DELIVERY_CHANNEL_CONCURRENCY",
    "RETRY_BASE_SECONDS",
    "EVENT_RETENTION_DAYS",
];
#[derive(Serialize, Deserialize)]
pub struct Metadata {
    version: u32,
    relayix_version: String,
    database_version: i32,
    settings: BTreeMap<String, String>,
}
fn derive(password: &str, salt: &[u8]) -> Result<[u8; 32]> {
    if !(12..=1024).contains(&password.len()) {
        return Err(anyhow!("Use a backup password of 12–1024 UTF-8 bytes"));
    }
    let params =
        Params::new(65536, 3, 1, Some(32)).map_err(|_| anyhow!("Invalid backup KDF parameters"))?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|_| anyhow!("Unable to derive backup key"))?;
    Ok(key)
}
fn aad(header: &[u8], index: u64, kind: u8, length: u32) -> Vec<u8> {
    let mut data = header.to_vec();
    data.extend_from_slice(&index.to_be_bytes());
    data.push(kind);
    data.extend_from_slice(&length.to_be_bytes());
    data
}
fn frame(
    writer: &mut impl Write,
    key: &[u8; 32],
    header: &[u8; 36],
    index: u64,
    kind: u8,
    plain: &[u8],
) -> Result<()> {
    if plain.len() > CHUNK {
        return Err(anyhow!("Backup frame is too large"));
    }
    let mut nonce = [0u8; 12];
    nonce[..4].copy_from_slice(&header[32..36]);
    nonce[4..].copy_from_slice(&index.to_be_bytes());
    let length = (plain.len() + 28) as u32;
    let packet = secrets::encrypt_nonce(key, &nonce, &aad(header, index, kind, length), plain)?;
    writer.write_all(&[kind])?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&packet)?;
    Ok(())
}
fn read_frame(
    reader: &mut impl Read,
    key: &[u8; 32],
    header: &[u8; 36],
    index: u64,
) -> Result<(u8, Vec<u8>)> {
    let mut prefix = [0u8; 5];
    reader
        .read_exact(&mut prefix)
        .map_err(|_| anyhow!("Backup is truncated or incomplete"))?;
    let size = u32::from_be_bytes(
        prefix[1..]
            .try_into()
            .map_err(|_| anyhow!("Invalid backup frame"))?,
    );
    if !(28..=(CHUNK + 28) as u32).contains(&size) || prefix[0] > 2 {
        return Err(anyhow!("Invalid backup frame"));
    }
    let mut packet = vec![0u8; size as usize];
    reader
        .read_exact(&mut packet)
        .map_err(|_| anyhow!("Backup is truncated or incomplete"))?;
    let plain = secrets::decrypt(key, &aad(header, index, prefix[0], size), &packet)
        .map_err(|_| anyhow!("Wrong backup password or damaged backup"))?;
    Ok((prefix[0], plain))
}
fn seal(
    reader: &mut impl Read,
    writer: &mut impl Write,
    password: &str,
    metadata: &Metadata,
) -> Result<()> {
    let mut header = [0u8; 36];
    header[..16].copy_from_slice(MAGIC);
    let salt: [u8; 16] = rand::random();
    let prefix: [u8; 4] = rand::random();
    header[16..32].copy_from_slice(&salt);
    header[32..].copy_from_slice(&prefix);
    let key = derive(password, &salt)?;
    let meta = serde_json::to_vec(metadata)?;
    if meta.len() > CHUNK {
        return Err(anyhow!("Backup settings exceed the metadata limit"));
    }
    writer.write_all(&header)?;
    frame(writer, &key, &header, 0, 0, &meta)?;
    let mut buffer = vec![0u8; CHUNK];
    let mut index = 1u64;
    loop {
        let mut count = 0;
        while count < CHUNK {
            match reader.read(&mut buffer[count..]) {
                Ok(0) => break,
                Ok(n) => count += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(anyhow!("Database backup input failed")),
            }
        }
        if count == 0 {
            break;
        }
        frame(writer, &key, &header, index, 1, &buffer[..count])?;
        index = index
            .checked_add(1)
            .ok_or_else(|| anyhow!("Backup is too large"))?;
    }
    frame(writer, &key, &header, index, 2, &[])?;
    writer.flush()?;
    Ok(())
}
fn open(reader: &mut impl Read, writer: &mut impl Write, password: &str) -> Result<()> {
    let mut header = [0u8; 36];
    reader
        .read_exact(&mut header)
        .map_err(|_| anyhow!("Invalid backup header"))?;
    if &header[..16] != MAGIC {
        return Err(anyhow!("Unsupported backup format"));
    }
    let key = derive(password, &header[16..32])?;
    let (kind, bytes) = read_frame(reader, &key, &header, 0)?;
    if kind != 0 {
        return Err(anyhow!("Backup metadata is missing"));
    }
    let metadata: Metadata =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("Invalid backup metadata"))?;
    if metadata.version != 1
        || metadata
            .settings
            .keys()
            .any(|name| !SETTINGS.contains(&name.as_str()))
        || metadata
            .settings
            .values()
            .any(|v| v.contains(['\0', '\n', '\r']))
    {
        return Err(anyhow!("Unsupported backup settings"));
    }
    let version = |value: &str| -> Option<Vec<u32>> {
        let v = value
            .split('.')
            .map(str::parse::<u32>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .ok()?;
        (v.len() == 3).then_some(v)
    };
    if version(&metadata.relayix_version).is_none()
        || version(&metadata.relayix_version) > version(env!("CARGO_PKG_VERSION"))
    {
        return Err(anyhow!(
            "Upgrade Relayix before restoring a backup from a newer release"
        ));
    }
    let mut staged = tempfile::tempfile()
        .map_err(|_| anyhow!("Unable to create private backup verification storage"))?;
    let mut index = 1u64;
    loop {
        let (kind, plain) = read_frame(reader, &key, &header, index)?;
        match kind {
            1 => staged.write_all(&plain)?,
            2 if plain.is_empty() => break,
            _ => return Err(anyhow!("Invalid backup frame sequence")),
        }
        index = index
            .checked_add(1)
            .ok_or_else(|| anyhow!("Backup is too large"))?;
    }
    let mut trailing = [0u8; 1];
    if reader.read(&mut trailing)? != 0 {
        return Err(anyhow!("Unexpected trailing backup data"));
    }
    // Release no plaintext or settings until every frame and the final marker is authenticated.
    staged.seek(SeekFrom::Start(0))?;
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(&bytes)?;
    std::io::copy(&mut staged, writer)?;
    writer.flush()?;
    Ok(())
}
pub async fn run(mode: &str) -> Result<()> {
    let password = env::var("RELAYIX_BACKUP_PASSWORD").map_err(|_| {
        anyhow!("Backup password must be provided through the protected command environment")
    })?;
    if mode == "seal" {
        let db = database::connect().await?;
        let database_version: i32 =
            sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
                .fetch_one(&db)
                .await?;
        let metadata = Metadata {
            version: 1,
            relayix_version: env!("CARGO_PKG_VERSION").into(),
            database_version,
            settings: SETTINGS
                .iter()
                .filter_map(|name| env::var(name).ok().map(|value| ((*name).to_owned(), value)))
                .collect(),
        };
        seal(
            &mut std::io::stdin().lock(),
            &mut std::io::stdout().lock(),
            &password,
            &metadata,
        )?;
        let mut tx = db.begin().await?;
        audit::system(&mut tx, "backup.export", 1).await?;
        tx.commit().await?;
        Ok(())
    } else if mode == "open" {
        open(
            &mut std::io::stdin().lock(),
            &mut std::io::stdout().lock(),
            &password,
        )
    } else {
        Err(anyhow!("Use the operational backup command"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backups_authenticate_the_entire_stream_before_releasing_anything() {
        let password = key();
        let master = key();
        let bytes = vec![0u8; CHUNK * 2 + 17];
        let metadata = Metadata {
            version: 1,
            relayix_version: env!("CARGO_PKG_VERSION").into(),
            database_version: 170011,
            settings: BTreeMap::from([("SECRETS_KEY".into(), master.clone())]),
        };
        let mut encrypted = Vec::new();
        seal(&mut bytes.as_slice(), &mut encrypted, &password, &metadata).unwrap();
        assert!(!encrypted
            .windows(master.len())
            .any(|w| w == master.as_bytes()));
        let mut output = Vec::new();
        open(&mut encrypted.as_slice(), &mut output, &password).unwrap();
        let meta_len = u32::from_be_bytes(output[..4].try_into().unwrap()) as usize;
        assert_eq!(&output[4 + meta_len..], bytes.as_slice());
        for end in [0, 35, encrypted.len() - 1] {
            let mut result = Vec::new();
            assert!(open(&mut &encrypted[..end], &mut result, &password).is_err());
            assert!(result.is_empty());
        }
        let mut tampered = encrypted.clone();
        let i = tampered.len() / 2;
        tampered[i] ^= 1;
        let mut result = Vec::new();
        assert!(open(&mut tampered.as_slice(), &mut result, &password).is_err());
        assert!(result.is_empty());
        assert!(open(&mut encrypted.as_slice(), &mut result, &key()).is_err());
        assert!(result.is_empty());
        let mut second = Vec::new();
        seal(&mut bytes.as_slice(), &mut second, &password, &metadata).unwrap();
        assert_ne!(encrypted, second);
    }
}
