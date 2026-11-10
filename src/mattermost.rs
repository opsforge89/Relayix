use crate::*;

pub fn is_bot(cfg: &Value) -> bool {
    cfg["mode"].as_str() == Some("bot")
}
fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|c| c.is_ascii_alphanumeric())
}
pub fn validate(cfg: &Value) -> Result<()> {
    match cfg["mode"].as_str().unwrap_or("webhook") {
        "webhook" => return Ok(()),
        "bot" => {}
        _ => return Err(anyhow!("Mattermost mode must be webhook or bot")),
    }
    if cfg["bot_token"].as_str().unwrap_or("").trim().is_empty() {
        return Err(anyhow!("Missing bot or personal access token"));
    }
    let url = url::Url::parse(cfg["url"].as_str().unwrap_or(""))
        .map_err(|_| anyhow!("Invalid Mattermost server URL"))?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(anyhow!(
            "Mattermost server URL must not include a query or fragment"
        ));
    }
    match cfg["target"].as_str().unwrap_or("channel") {
        "channel" => {
            if !identifier(cfg["channel_id"].as_str().unwrap_or("")) {
                return Err(anyhow!(
                    "Mattermost channel ID is required; copy the ID from channel information"
                ));
            }
        }
        "direct" => {
            let recipient = cfg["recipient"]
                .as_str()
                .unwrap_or("")
                .trim()
                .trim_start_matches('@');
            if recipient.is_empty()
                || recipient.len() > 64
                || !recipient
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            {
                return Err(anyhow!(
                    "Provide a Mattermost recipient as @username or user ID"
                ));
            }
        }
        _ => return Err(anyhow!("Mattermost destination must be channel or direct")),
    }
    Ok(())
}
pub(crate) fn api_url(cfg: &Value, parts: &[&str]) -> Result<url::Url> {
    let mut url = url::Url::parse(cfg["url"].as_str().unwrap())?;
    let already_api = url.path().trim_end_matches('/').ends_with("/api/v4");
    let mut path = url
        .path_segments_mut()
        .map_err(|_| anyhow!("Invalid Mattermost server URL"))?;
    path.pop_if_empty();
    if !already_api {
        path.extend(["api", "v4"]);
    }
    path.extend(parts);
    drop(path);
    Ok(url)
}
async fn response_json(response: reqwest::Response) -> Result<Value> {
    let response = retry_policy::response(response).await?;
    response
        .json()
        .await
        .map_err(|_| anyhow!("Mattermost API returned an invalid response"))
}
fn response_id(value: &Value) -> Result<&str> {
    value["id"]
        .as_str()
        .filter(|id| identifier(id))
        .ok_or_else(|| anyhow!("Mattermost API did not return a valid ID"))
}
async fn destination(client: &reqwest::Client, cfg: &Value) -> Result<String> {
    validate(cfg)?;
    let token = cfg["bot_token"].as_str().unwrap();
    let channel = if cfg["target"].as_str().unwrap_or("channel") == "direct" {
        let me = response_json(
            client
                .get(api_url(cfg, &["users", "me"])?)
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| anyhow!(e.without_url()))?,
        )
        .await?;
        let sender = response_id(&me)?;
        let raw = cfg["recipient"].as_str().unwrap().trim();
        let recipient = if !raw.starts_with('@') && raw.len() == 26 && identifier(raw) {
            raw.to_owned()
        } else {
            let user = response_json(
                client
                    .get(api_url(
                        cfg,
                        &["users", "username", raw.trim_start_matches('@')],
                    )?)
                    .bearer_auth(token)
                    .send()
                    .await
                    .map_err(|e| anyhow!(e.without_url()))?,
            )
            .await?;
            response_id(&user)?.to_owned()
        };
        let direct = response_json(
            client
                .post(api_url(cfg, &["channels", "direct"])?)
                .bearer_auth(token)
                .json(&json!([sender, recipient]))
                .send()
                .await
                .map_err(|e| anyhow!(e.without_url()))?,
        )
        .await?;
        response_id(&direct)?.to_owned()
    } else {
        cfg["channel_id"].as_str().unwrap().to_owned()
    };
    Ok(channel)
}
pub async fn send(client: &reqwest::Client, cfg: &Value, text: &str) -> Result<()> {
    let channel=destination(client,cfg).await?;
    let token=cfg["bot_token"].as_str().unwrap();
    let post = response_json(
        client
            .post(api_url(cfg, &["posts"])?)
            .bearer_auth(token)
            .json(&json!({"channel_id":channel,"message":text}))
            .send()
            .await
            .map_err(|e| anyhow!(e.without_url()))?,
    )
    .await?;
    response_id(&post)?;
    Ok(())
}

pub async fn send_files(context:&attachments::Delivery<'_>,text:&str)->Result<()> {
    use reqwest::multipart::{Form,Part};
    if context.state("posted").await?.is_some() {return Ok(());}
    let cfg=context.cfg;
    let token=cfg["bot_token"].as_str().unwrap();
    let channel=match context.state("channel").await? {
        Some(channel)=>channel,
        None=>{let channel=destination(context.client,cfg).await?;context.record("channel",&channel).await?;channel}
    };
    let mut ids=Vec::new();
    for (position,file) in context.files.iter().enumerate() {
        let step=format!("upload-{position}");
        let id=match context.state(&step).await? {
            Some(id)=>id,
            None=>{
                let form=Form::new().text("channel_id",channel.clone()).part("files",Part::bytes(file.data.clone()).file_name(file.name.clone()).mime_str(&file.content_type)?);
                let response=response_json(context.client.post(api_url(cfg,&["files"])?).bearer_auth(token).timeout(Duration::from_secs(45)).multipart(form).send().await.map_err(|e|anyhow!(e.without_url()))?).await?;
                let infos=response["file_infos"].as_array().filter(|v|v.len()==1).ok_or_else(||anyhow!("Mattermost did not return the uploaded file"))?;
                let id=response_id(&infos[0])?.to_owned();
                context.record(&step,&id).await?;
                id
            }
        };
        ids.push(id);
    }
    let post=response_json(context.client.post(api_url(cfg,&["posts"])?).bearer_auth(token).json(&json!({"channel_id":channel,"message":text,"file_ids":ids})).send().await.map_err(|e|anyhow!(e.without_url()))?).await?;
    context.record("posted",response_id(&post)?).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bot_destinations_are_validated_and_subpaths_preserved() {
        let mut cfg = json!({"mode":"bot","url":"https://chat.example.org/team/","bot_token":key(),"target":"channel","channel_id":"testchannel"});
        assert!(validate(&cfg).is_ok());
        assert_eq!(
            api_url(&cfg, &["posts"]).unwrap().as_str(),
            "https://chat.example.org/team/api/v4/posts"
        );
        cfg["url"] = json!("https://chat.example.org/team/api/v4/");
        assert_eq!(
            api_url(&cfg, &["posts"]).unwrap().as_str(),
            "https://chat.example.org/team/api/v4/posts"
        );
        cfg["target"] = json!("direct");
        assert!(validate(&cfg).is_err());
        cfg["recipient"] = json!("@ops-bot");
        assert!(validate(&cfg).is_ok());
        cfg["recipient"] = json!("../ops");
        assert!(validate(&cfg).is_err());
        cfg["mode"] = json!("invalid");
        assert!(validate(&cfg).is_err());
    }
}
