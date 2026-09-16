use std::time::Duration;

use serde_json::Value;

use crate::error::StoreError;

pub const ANTHROPIC_API_KEY_ABSENT: &str = "ANTHROPIC_API_KEY is absent. The key is read only from that environment variable. No network call will be made. The key is not prompted for and is not written to any file or config directory.";
pub const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-5";

pub fn anthropic_api_key_from_env() -> Option<String> {
    match std::env::var("ANTHROPIC_API_KEY") {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => None,
    }
}

pub fn anthropic_model_from_env() -> String {
    match std::env::var("ANTHROPIC_MODEL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => DEFAULT_ANTHROPIC_MODEL.to_string(),
    }
}

fn redact(key: &str, text: &str) -> String {
    if key.is_empty() {
        text.to_string()
    } else {
        text.replace(key, "REDACTED")
    }
}

pub fn encode_base64(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        i += 3;
    }
    let rem = data.len() - i;
    if rem == 1 {
        let n = (data[i] as u32) << 16;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push('=');
    }
    out
}

pub struct AnthropicClient {
    api_key: String,
}

impl AnthropicClient {
    pub fn from_env() -> Result<Self, StoreError> {
        match anthropic_api_key_from_env() {
            Some(api_key) => Ok(Self { api_key }),
            None => Err(StoreError::Anthropic(ANTHROPIC_API_KEY_ABSENT.to_string())),
        }
    }

    pub fn post_messages(&self, body: &Value) -> Result<Value, StoreError> {
        let payload = serde_json::to_string(body)
            .map_err(|err| StoreError::Anthropic(err.to_string()))?;
        match ureq::post(ANTHROPIC_MESSAGES_URL)
            .set("x-api-key", &self.api_key)
            .set("anthropic-version", ANTHROPIC_VERSION)
            .set("content-type", "application/json")
            .timeout(Duration::from_secs(180))
            .send_string(&payload)
        {
            Ok(response) => {
                let status = response.status();
                let text = response
                    .into_string()
                    .map_err(|err| StoreError::Anthropic(redact(&self.api_key, &err.to_string())))?;
                if !(200..300).contains(&status) {
                    return Err(StoreError::Anthropic(format!(
                        "status={status} body={}",
                        redact(&self.api_key, &text)
                    )));
                }
                serde_json::from_str(&text).map_err(|err| {
                    StoreError::Anthropic(redact(&self.api_key, &err.to_string()))
                })
            }
            Err(ureq::Error::Status(status, response)) => {
                let text = response.into_string().unwrap_or_default();
                Err(StoreError::Anthropic(format!(
                    "status={status} body={}",
                    redact(&self.api_key, &text)
                )))
            }
            Err(err) => Err(StoreError::Anthropic(redact(&self.api_key, &err.to_string()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::encode_base64;

    #[test]
    fn encode_base64_known_vectors() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"Man"), "TWFu");
        assert_eq!(encode_base64(b"Ma"), "TWE=");
        assert_eq!(encode_base64(b"M"), "TQ==");
    }
}
