//! Typed adapter inputs; storage and the public API retain their JSON format.
use crate::*;
#[derive(Deserialize)]
#[serde(tag = "kind", content = "config", rename_all = "snake_case")]
pub enum Config {
    Telegram {
        bot_token: String,
        chat_id: String,
    },
    Ntfy {
        url: String,
        topic: String,
    },
    Gotify {
        url: String,
        token: String,
    },
    Pushover {
        token: String,
        user: String,
    },
    Matrix {
        url: String,
        room_id: String,
        access_token: String,
    },
    Smtp {
        host: String,
        from: String,
        to: String,
    },
    Mattermost {
        url: String,
    },
    Slack {
        url: String,
    },
    Discord {
        url: String,
    },
    Teams {
        url: String,
    },
    GoogleChat {
        url: String,
    },
    RocketChat {
        url: String,
    },
    Webhook {
        url: String,
    },
}
impl Config {
    pub fn parse(kind: &str, config: &Value) -> Result<Self> {
        let typed: Self = serde_json::from_value(json!({"kind":kind,"config":config}))
            .map_err(|e| anyhow!("Invalid channel configuration: {e}"))?;
        let fields: Vec<&str> = match &typed {
            Self::Telegram { bot_token, chat_id } => vec![bot_token, chat_id],
            Self::Ntfy { url, topic } => vec![url, topic],
            Self::Gotify { url, token } => vec![url, token],
            Self::Pushover { token, user } => vec![token, user],
            Self::Matrix {
                url,
                room_id,
                access_token,
            } => vec![url, room_id, access_token],
            Self::Smtp { host, from, to } => vec![host, from, to],
            Self::Mattermost { url }
            | Self::Slack { url }
            | Self::Discord { url }
            | Self::Teams { url }
            | Self::GoogleChat { url }
            | Self::RocketChat { url }
            | Self::Webhook { url } => vec![url],
        };
        if fields.iter().any(|field| field.trim().is_empty()) {
            return Err(anyhow!("Required channel fields must not be empty"));
        }
        Ok(typed)
    }
    pub fn url(&self) -> Option<&str> {
        match self {
            Self::Ntfy { url, .. }
            | Self::Gotify { url, .. }
            | Self::Matrix { url, .. }
            | Self::Mattermost { url }
            | Self::Slack { url }
            | Self::Discord { url }
            | Self::Teams { url }
            | Self::GoogleChat { url }
            | Self::RocketChat { url }
            | Self::Webhook { url } => Some(url),
            _ => None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_adapter_inputs_never_reach_send() {
        for (kind, cfg) in [
            ("unknown", json!({"url":"https://example.org"})),
            ("gotify", json!({"url":"https://example.org","token":7})),
            ("telegram", json!({"bot_token":"token","chat_id":null})),
            (
                "smtp",
                json!({"host":"smtp.example.org","from":"a@b.c","to":" "}),
            ),
        ] {
            assert!(Config::parse(kind, &cfg).is_err());
        }
    }
}
