//! Shared release/build validation. An absent configuration disables updates.
use base64::Engine;
use serde_json::Value;

pub fn validate(config: Option<&Value>) -> Result<bool, &'static str> {
    let Some(config) = config else {
        return Ok(false);
    };
    let key = config
        .get("pubkey")
        .and_then(Value::as_str)
        .ok_or("updater requires a public key in the release config overlay")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(key)
        .map_err(|_| "invalid updater public key encoding")?;
    let decoded = std::str::from_utf8(&decoded).map_err(|_| "invalid updater public key text")?;
    minisign_verify::PublicKey::decode(decoded).map_err(|_| "invalid updater public key format")?;
    let Some(endpoints) = config.get("endpoints").and_then(Value::as_array) else {
        return Err("updater requires HTTPS endpoints in the release config overlay");
    };
    if endpoints.is_empty()
        || endpoints.iter().any(|url| {
            !url.as_str()
                .and_then(|value| url::Url::parse(value).ok())
                .is_some_and(|url| {
                    url.scheme() == "https"
                        && url.host_str().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                        && url.fragment().is_none()
                })
        })
    {
        return Err("updater requires nonempty HTTPS endpoints");
    }
    if config.get("requireSignedVersion").and_then(Value::as_bool) != Some(true) {
        return Err("updater requires signed-version verification with CLI 2.12.0");
    }
    for flag in [
        "dangerousInsecureTransportProtocol",
        "dangerous-insecure-transport-protocol",
        "dangerousAcceptInvalidCerts",
        "dangerous-accept-invalid-certs",
        "dangerousAcceptInvalidHostnames",
        "dangerous-accept-invalid-hostnames",
        "allowDowngrades",
        "allow-downgrades",
    ] {
        if config.get(flag).and_then(Value::as_bool) == Some(true) {
            return Err("unsafe updater settings are forbidden");
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(endpoint: &str) -> Value {
        // Public test key from minisign-verify's documented example. Never a
        // release trust key; no private key is created or stored.
        let key = base64::engine::general_purpose::STANDARD.encode("untrusted comment: test key\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3\n");
        json!({"pubkey": key, "endpoints":[endpoint], "requireSignedVersion":true})
    }

    #[test]
    fn absent_release_config_is_disabled() {
        assert_eq!(validate(None), Ok(false));
    }

    #[test]
    fn incomplete_or_insecure_config_is_rejected() {
        for value in [
            json!({}),
            json!({"pubkey":"", "endpoints":["https://updates.example/latest.json"]}),
            json!({"pubkey":"provided-at-release", "endpoints":[]}),
            json!({"pubkey":"provided-at-release", "endpoints":["http://updates.example/latest.json"]}),
        ] {
            assert!(validate(Some(&value)).is_err());
        }
    }

    #[test]
    fn static_and_server_endpoints_use_same_configuration() {
        for endpoint in [
            "https://updates.example/latest.json",
            "https://updates.example/{{target}}/{{arch}}/{{current_version}}",
        ] {
            let value = config(endpoint);
            assert_eq!(validate(Some(&value)), Ok(true));
        }
    }

    #[test]
    fn release_cannot_disable_signature_version_binding_or_tls() {
        let mut value = config("https://updates.example/latest.json");
        value["requireSignedVersion"] = json!(false);
        assert!(validate(Some(&value)).is_err());
        value["requireSignedVersion"] = json!(true);
        for flag in [
            "dangerousInsecureTransportProtocol",
            "dangerous-insecure-transport-protocol",
            "dangerousAcceptInvalidCerts",
            "dangerous-accept-invalid-certs",
            "dangerousAcceptInvalidHostnames",
            "dangerous-accept-invalid-hostnames",
            "allowDowngrades",
            "allow-downgrades",
        ] {
            let mut unsafe_value = value.clone();
            unsafe_value[flag] = json!(true);
            assert!(validate(Some(&unsafe_value)).is_err());
        }
    }

    #[test]
    fn malformed_keys_urls_and_embedded_credentials_are_rejected() {
        let mut bad_key = config("https://updates.example/latest.json");
        bad_key["pubkey"] = json!("not-a-public-key");
        assert!(validate(Some(&bad_key)).is_err());
        bad_key["pubkey"] =
            json!(base64::engine::general_purpose::STANDARD.encode("not a minisign key"));
        assert!(validate(Some(&bad_key)).is_err());
        for endpoint in [
            "https://",
            "https://[invalid",
            "http://updates.example",
            "https://user:password@updates.example/latest.json",
            "https://updates.example/latest.json#fragment",
        ] {
            assert!(validate(Some(&config(endpoint))).is_err());
        }
    }
}
