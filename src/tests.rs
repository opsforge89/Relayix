use super::*;

#[test]
fn monitoring_periods_are_bounded_and_precise() {
    for (period, seconds) in [
        ("5m", 300),
        ("15m", 900),
        ("30m", 1800),
        ("1h", 3600),
        ("24h", 86400),
        ("7d", 604800),
        ("30d", 2592000),
    ] {
        assert_eq!(monitoring_period(period).unwrap().num_seconds(), seconds);
    }
    assert!(monitoring_period("all").is_none());
    assert!(monitoring_period("-5m").is_none());
}

#[test]
fn generated_endpoints_are_valid_and_unique() {
    for name in [
        "Deploy alerts",
        "Alerts \u{2603}",
        "",
        "   ",
        "a/b?secret=1",
        &"Very long name ".repeat(20),
    ] {
        let first = generated_slug(name, Uuid::new_v4());
        let second = generated_slug(name, Uuid::new_v4());
        assert!(slug_valid(&first));
        assert!(slug_valid(&second));
        assert_ne!(first, second);
    }
}

#[test]
fn password_policy_rejects_weak_passwords() {
    let strong = format!("A7!b{}", key());
    assert!(auth::password_valid(&strong, "operator").is_ok());
    let invalid = vec![
        "x".to_owned(),
        format!("{}!", key().to_lowercase()),
        format!("{}!", key().to_uppercase()),
        format!("Ab!{}", "x".repeat(20)),
        format!("Ab7{}", "x".repeat(20)),
        format!("Operator7!{}", key()),
        format!("Password7!{}", key()),
        format!("Qwerty7!{}", key()),
    ];
    for password in invalid {
        assert!(auth::password_valid(&password, "operator").is_err());
    }
}

#[test]
fn telegram_topics_are_optional_positive_integers() {
    let mut cfg = json!({"bot_token":key(),"chat_id":"-100123"});
    assert!(telegram_payload(&cfg, "test")
        .unwrap()
        .get("message_thread_id")
        .is_none());
    cfg["message_thread_id"] = json!("42");
    assert_eq!(
        telegram_payload(&cfg, "test").unwrap()["message_thread_id"],
        42
    );
    assert_eq!(
        telegram_payload(&cfg, "<b>safe</b>").unwrap()["parse_mode"],
        "HTML"
    );
    for invalid in [
        json!(0),
        json!(-3),
        json!("oops"),
        json!(2147483648_i64),
        json!(1.5),
    ] {
        cfg["message_thread_id"] = invalid;
        assert!(valid_config("telegram", &cfg, false).is_err());
    }
}

fn smtp() -> Value {
    json!({"host":"smtp.example.org","from":"Relayix <alerts@example.org>","to":"ops@example.org",
        "port":"587","tls":"starttls","username":"bot","password":key()})
}

#[test]
fn smtp_requires_valid_mailboxes_port_and_paired_credentials() {
    assert!(valid_config("smtp", &smtp(), false).is_ok());
    for (field, invalid) in [
        ("host", json!("https://smtp.example.org")),
        ("host", json!("smtp.example.org/path")),
        ("from", json!("invalid")),
        ("to", json!("a@b.org,c@d.org")),
        ("port", json!(0)),
        ("port", json!(65536)),
        ("port", json!("abc")),
        ("tls", json!("opportunistic")),
        ("password", json!("")),
        ("username", json!("")),
    ] {
        let mut cfg = smtp();
        cfg[field] = invalid;
        assert!(
            valid_config("smtp", &cfg, false).is_err(),
            "accepted invalid {field}"
        );
    }
    let mut relay = smtp();
    relay["username"] = json!("");
    relay["password"] = json!("");
    assert!(valid_config("smtp", &relay, false).is_ok());
    relay["tls"] = json!("none");
    assert!(valid_config("smtp", &relay, false).is_err());
    assert!(valid_config("smtp", &relay, true).is_ok());
    relay.as_object_mut().unwrap().remove("port");
    relay["tls"] = json!("tls");
    assert_eq!(providers::smtp_port(&relay).unwrap(), 465);
}

#[test]
fn matrix_validates_room_and_transport() {
    let cfg = json!({"url":"https://matrix.example.org", "room_id":"!abc:example.org", "access_token":key()});
    assert!(valid_config("matrix", &cfg, false).is_ok());
    for (field, invalid) in [
        ("room_id", "#alias:example.org"),
        ("access_token", ""),
        ("url", "http://matrix.example.org"),
        ("url", "https://matrix.example.org?token=secret"),
        ("url", "https://user:pass@matrix.example.org"),
        ("url", "https://matrix.example.org/#room"),
    ] {
        let mut invalid_cfg = cfg.clone();
        invalid_cfg[field] = json!(invalid);
        assert!(
            valid_config("matrix", &invalid_cfg, false).is_err(),
            "accepted invalid {field}"
        );
    }
    assert!(valid_config("smtp", &json!([]), false).is_err());
    assert!(valid_config("matrix", &json!(null), false).is_err());
}

#[test]
fn secrets_survive_masked_edits_and_can_be_replaced_or_cleared() {
    for old in [
        smtp(),
        json!({"url":"https://matrix.example.org","room_id":"!abc:example.org","access_token":key()}),
        json!({"token":key(),"user":key()}),
        json!({"url":"https://example.org/private-webhook"}),
    ] {
        let mut masked = old.clone();
        mask(&mut masked);
        for &field in providers::SECRET_FIELDS {
            if old.get(field).is_some() {
                assert_eq!(masked[field], "********");
            }
        }
        restore_masked(&mut masked, &old, "smtp", "smtp").unwrap();
        assert_eq!(masked, old);
    }
    let replacement = key();
    let mut edited = smtp();
    edited["password"] = json!(replacement);
    restore_masked(&mut edited, &smtp(), "smtp", "smtp").unwrap();
    assert_eq!(edited["password"], replacement);
    edited["password"] = json!("");
    restore_masked(&mut edited, &smtp(), "smtp", "smtp").unwrap();
    assert_eq!(edited["password"], "");
}

#[test]
fn smtp_sender_names_preserve_legacy_mailboxes_and_reject_header_injection() {
    let mut cfg = smtp();
    assert_eq!(
        providers::smtp_sender(&cfg).unwrap().name.as_deref(),
        Some("Relayix")
    );
    cfg["from_name"] = json!("  Relayix Alerts \u{2709}  ");
    let sender = providers::smtp_sender(&cfg).unwrap();
    assert_eq!(sender.name.as_deref(), Some("Relayix Alerts \u{2709}"));
    assert_eq!(sender.email.to_string(), "alerts@example.org");
    for name in [
        json!("bad\r\nBcc: evil@example.org"),
        json!("x".repeat(101)),
        json!(42),
    ] {
        cfg["from_name"] = name;
        assert!(valid_config("smtp", &cfg, false).is_err());
    }
    cfg["from_name"] = json!("");
    assert_eq!(
        providers::smtp_sender(&cfg).unwrap().name.as_deref(),
        Some("Relayix")
    );
}
