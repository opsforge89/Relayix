//! Versioned authenticated encryption for stored credentials; keys never live in the database.
use crate::*;
use aes_gcm::{aead::{Aead,Payload,consts::U12},Aes256Gcm,KeyInit,Nonce};
use base64::{engine::general_purpose::{STANDARD,URL_SAFE_NO_PAD},Engine};
use std::sync::OnceLock;
static KEY:OnceLock<Option<[u8;32]>>=OnceLock::new();
pub fn parse_key(value:&str)->Result<[u8;32]> {
    let bytes=if value.len()==64 {hex::decode(value).ok()} else {STANDARD.decode(value).ok().or_else(||URL_SAFE_NO_PAD.decode(value).ok())};
    bytes.and_then(|v|v.try_into().ok()).ok_or_else(||anyhow!("SECRETS_KEY must contain 32 random bytes encoded as base64 or 64 hexadecimal characters"))
}
pub fn configure()->Result<()> {
    let raw=env::var("SECRETS_KEY").unwrap_or_default();
    let key=if raw.trim().is_empty() {None} else {Some(parse_key(raw.trim())?)};
    KEY.set(key).map_err(|_|anyhow!("Secret storage was already configured"))
}
pub fn enabled()->bool {KEY.get().is_some_and(Option::is_some)}
fn key()->Result<[u8;32]> {KEY.get().copied().flatten().ok_or_else(||anyhow!("SECRETS_KEY is required for this encrypted database"))}
pub fn encrypt(key:&[u8;32],aad:&[u8],plain:&[u8])->Result<Vec<u8>> {
    let nonce_bytes: [u8;12]=rand::random();
    encrypt_nonce(key,&nonce_bytes,aad,plain)
}
pub fn encrypt_nonce(key:&[u8;32],nonce:&[u8;12],aad:&[u8],plain:&[u8])->Result<Vec<u8>> {
    let cipher=Aes256Gcm::new_from_slice(key).map_err(|_|anyhow!("Invalid encryption key"))?;
    let nonce_value=Nonce::<U12>::from(*nonce);
    let encrypted=cipher.encrypt(&nonce_value,Payload{msg:plain,aad}).map_err(|_|anyhow!("Secret encryption failed"))?;
    let mut output=nonce.to_vec();output.extend_from_slice(&encrypted);Ok(output)
}
pub fn decrypt(key:&[u8;32],aad:&[u8],packet:&[u8])->Result<Vec<u8>> {
    if packet.len()<28 {return Err(anyhow!("Invalid encrypted data"));}
    let raw:[u8;12]=packet[..12].try_into().map_err(|_|anyhow!("Invalid encrypted data"))?;
    let cipher=Aes256Gcm::new_from_slice(key).map_err(|_|anyhow!("Invalid encryption key"))?;
    cipher.decrypt(&Nonce::<U12>::from(raw),Payload{msg:&packet[12..],aad}).map_err(|_|anyhow!("Encryption key is incorrect or encrypted data was modified"))
}
fn seal_with(key:&[u8;32],aad:&str,value:&[u8])->Result<String> {Ok(format!("v1:{}",STANDARD.encode(encrypt(key,aad.as_bytes(),value)?)))}
fn open_with(key:&[u8;32],aad:&str,value:&str)->Result<Vec<u8>> {
    let packet=value.strip_prefix("v1:").and_then(|s|STANDARD.decode(s).ok()).ok_or_else(||anyhow!("Invalid encrypted secret format"))?;
    decrypt(key,aad.as_bytes(),&packet)
}
fn protected()->impl Iterator<Item=&'static str> {providers::SECRET_FIELDS.iter().copied().chain(std::iter::once("body_template"))}
fn canonical(value:&Value)->Value {
    match value {
        Value::Object(fields)=>{
            let mut keys=fields.keys().collect::<Vec<_>>();keys.sort();let mut result=serde_json::Map::new();
            for name in keys {result.insert(name.clone(),canonical(&fields[name]));}Value::Object(result)
        }
        Value::Array(values)=>Value::Array(values.iter().map(canonical).collect()),
        Value::Number(number) if number.as_f64()==Some(0.0)=>json!(0),
        _=>value.clone(),
    }
}
fn context(id:Uuid,kind:&str,cfg:&Value)->String {
    let mut public=cfg.clone();if let Some(fields)=public.as_object_mut() {for field in protected() {fields.remove(field);}}
    format!("channel:{id}:{kind}:{}",hash(&canonical(&public).to_string()))
}
fn transform(key:&[u8;32],id:Uuid,kind:&str,cfg:&Value,opening:bool)->Result<Value> {
    let mut output=cfg.clone();let context=context(id,kind,cfg);
    for field in protected() {
        if let Some(value)=cfg.get(field).filter(|v|!v.is_null() && v.as_str()!=Some("")) {
            let aad=format!("{context}:{field}");
            output[field]=if opening {
                let encoded=value.as_str().ok_or_else(||anyhow!("Invalid encrypted channel secret"))?;
                serde_json::from_slice(&open_with(key,&aad,encoded)?).map_err(|_|anyhow!("Invalid encrypted channel secret"))?
            } else {json!(seal_with(key,&aad,&serde_json::to_vec(value)?)?)};
        }
    }
    Ok(output)
}
pub fn seal_config(id:Uuid,kind:&str,cfg:&Value)->Result<Value> {
    if !enabled() {return Ok(cfg.clone());}transform(&key()?,id,kind,cfg,false)
}
pub fn open_config(id:Uuid,kind:&str,cfg:&Value,encrypted:bool)->Result<Value> {
    if !encrypted {return Ok(cfg.clone());}transform(&key()?,id,kind,cfg,true)
}
pub fn seal_text(id:Uuid,field:&str,value:&str)->Result<String> {
    if !enabled() {return Ok(value.to_owned());}seal_with(&key()?,&format!("totp:{id}:{field}"),value.as_bytes())
}
pub fn open_text(id:Uuid,field:&str,value:&str,encrypted:bool)->Result<String> {
    if !encrypted {return Ok(value.to_owned());}
    String::from_utf8(open_with(&key()?,&format!("totp:{id}:{field}"),value)?).map_err(|_|anyhow!("Invalid encrypted authenticator secret"))
}
pub async fn migrate(db:&PgPool)->Result<()> {
    let mut tx=db.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(914029)").execute(&mut *tx).await?;
    let check:Option<String>=sqlx::query_scalar("SELECT key_check FROM secrets_state WHERE singleton FOR UPDATE").fetch_optional(&mut *tx).await?;
    if !enabled() {
        let encrypted:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE secrets_encrypted) OR EXISTS(SELECT 1 FROM user_totp WHERE secrets_encrypted)").fetch_one(&mut *tx).await?;
        if check.is_some() || encrypted {return Err(anyhow!("SECRETS_KEY is required; encrypted credentials were preserved"));}
        tx.commit().await?;
        tracing::warn!("SECRETS_KEY is unset; stored integration and authenticator credentials are not encrypted");
        return Ok(());
    }
    let master=key()?;
    if let Some(check)=check {
        let value=open_with(&master,"relayix-key-check/v1",&check)?;
        if value!=b"relayix-key-check/v1" {return Err(anyhow!("Invalid secret storage key check"));}
    }
    let mut count=0i64;let mut cursor:Option<Uuid>=None;
    loop {
        let rows:Vec<(Uuid,String,Value,bool)>=sqlx::query_as("SELECT id,kind,config,secrets_encrypted FROM channels WHERE $1::uuid IS NULL OR id>$1 ORDER BY id FOR UPDATE LIMIT 500").bind(cursor).fetch_all(&mut *tx).await?;
        if rows.is_empty() {break;}
        for (id,kind,cfg,encrypted) in rows {
            cursor=Some(id);
            if encrypted {open_config(id,&kind,&cfg,true)?;} else {
                let encoded=seal_config(id,&kind,&cfg)?;
                sqlx::query("UPDATE channels SET config=$2,secrets_encrypted=true WHERE id=$1").bind(id).bind(encoded).execute(&mut *tx).await?;count+=1;
            }
        }
    }
    cursor=None;
    loop {
        let rows:Vec<(Uuid,Option<String>,Option<String>,bool)>=sqlx::query_as("SELECT user_id,secret,pending_secret,secrets_encrypted FROM user_totp WHERE $1::uuid IS NULL OR user_id>$1 ORDER BY user_id FOR UPDATE LIMIT 500").bind(cursor).fetch_all(&mut *tx).await?;
        if rows.is_empty() {break;}
        for (id,secret,pending,encrypted) in rows {
            cursor=Some(id);
            if encrypted {
                if let Some(secret)=secret {open_text(id,"secret",&secret,true)?;}
                if let Some(pending)=pending {open_text(id,"pending",&pending,true)?;}
            } else {
                let secret=secret.map(|v|seal_text(id,"secret",&v)).transpose()?;
                let pending=pending.map(|v|seal_text(id,"pending",&v)).transpose()?;
                sqlx::query("UPDATE user_totp SET secret=$2,pending_secret=$3,secrets_encrypted=true WHERE user_id=$1").bind(id).bind(secret).bind(pending).execute(&mut *tx).await?;count+=1;
            }
        }
    }
    sqlx::query("INSERT INTO secrets_state(singleton,key_check) VALUES(true,$1) ON CONFLICT(singleton) DO NOTHING")
        .bind(seal_with(&master,"relayix-key-check/v1",b"relayix-key-check/v1")?).execute(&mut *tx).await?;
    if count>0 {audit::system(&mut tx,"secrets.migrate",count).await?;}
    tx.commit().await?;
    Ok(())
}
pub async fn state(State(s):State<AppState>,h:HeaderMap)->ApiResult<Value> {
    auth::require_admin(&h,&s).await?;Ok(Json(json!({"enabled":enabled(),"cipher":"AES-256-GCM","backups":true})))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_envelopes_bind_secrets_to_channel_and_destination() {
        let key:[u8;32]=rand::random();let id=Uuid::new_v4();
        let config=json!({"host":"smtp.example.org","port":587,"password":"stored-password","username":"operator","body_template":{"api_key":"private"}});
        let sealed=transform(&key,id,"smtp",&config,false).unwrap();
        assert!(!sealed.to_string().contains("stored-password"));
        assert_eq!(transform(&key,id,"smtp",&sealed,true).unwrap(),config);
        let mut altered=sealed.clone();altered["host"]=json!("different.example.org");
        assert!(transform(&key,id,"smtp",&altered,true).is_err());
        assert!(transform(&key,Uuid::new_v4(),"smtp",&sealed,true).is_err());
        assert!(transform(&key,id,"webhook",&sealed,true).is_err());
        assert!(transform(&rand::random(),id,"smtp",&sealed,true).is_err());
        assert_ne!(sealed,transform(&key,id,"smtp",&config,false).unwrap());
        assert!(parse_key("short").is_err());
        assert_eq!(parse_key(&STANDARD.encode(key)).unwrap(),key);
    }
}
