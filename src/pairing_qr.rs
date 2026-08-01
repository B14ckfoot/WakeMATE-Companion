use crate::types::PROTOCOL_VERSION;

/// Serializes the local, visual pairing contract carried directly by the QR
/// code. Network discovery is intentionally kept outside this pure builder so
/// every field and omission rule can be tested on every development platform.
pub(crate) fn payload_from_parts(
    token: &str,
    device_name: &str,
    api_port: u16,
    tls_port: u16,
    tls_fingerprint: &str,
    ip: Option<String>,
    mac: Option<String>,
) -> String {
    let mut payload = serde_json::json!({
        "v": 3,
        "kind": "wakemate-pairing",
        "name": device_name,
        "api_port": api_port,
        "tls_port": tls_port,
        "fp": tls_fingerprint,
        "token": token,
        "protocol_version": PROTOCOL_VERSION,
    });

    if let Some(ip) = ip {
        payload["ip"] = serde_json::Value::String(ip);
    }
    if let Some(mac) = mac {
        payload["mac"] = serde_json::Value::String(mac);
    }

    payload.to_string()
}

#[cfg(test)]
mod tests {
    use super::{payload_from_parts, PROTOCOL_VERSION};

    const TEST_TLS_FINGERPRINT: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn payload_is_structured_json_carrying_the_v3_contract_fields() {
        let payload = payload_from_parts(
            "token-value",
            "Desk Rig",
            7777,
            7778,
            TEST_TLS_FINGERPRINT,
            Some("192.168.1.50".to_string()),
            Some("00:11:22:33:44:55".to_string()),
        );

        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();

        assert_eq!(parsed["v"], 3);
        assert_eq!(parsed["kind"], "wakemate-pairing");
        assert_eq!(parsed["name"], "Desk Rig");
        assert_eq!(parsed["api_port"], 7777);
        assert_eq!(parsed["tls_port"], 7778);
        assert_eq!(parsed["fp"], TEST_TLS_FINGERPRINT);
        assert_eq!(parsed["token"], "token-value");
        assert_eq!(parsed["ip"], "192.168.1.50");
        assert_eq!(parsed["mac"], "00:11:22:33:44:55");
        assert_eq!(parsed["protocol_version"], PROTOCOL_VERSION);
    }

    #[test]
    fn payload_omits_unknown_network_fields() {
        let payload = payload_from_parts(
            "token-value",
            "Desk Rig",
            7777,
            7778,
            TEST_TLS_FINGERPRINT,
            None,
            None,
        );

        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();

        assert!(parsed.get("ip").is_none());
        assert!(parsed.get("mac").is_none());
        assert_eq!(parsed["token"], "token-value");
    }

    #[test]
    fn payload_preserves_special_characters_in_the_device_name() {
        // Device names are free text (e.g. "Ana's PC & Desk") and must
        // round-trip through the structured JSON payload unchanged.
        let payload = payload_from_parts(
            "token-value",
            "Ana's PC & Desk",
            7777,
            7778,
            TEST_TLS_FINGERPRINT,
            None,
            None,
        );

        let parsed: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed["name"], "Ana's PC & Desk");
    }
}
