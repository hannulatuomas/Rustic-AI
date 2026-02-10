use serde_json::Value;

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Text(String),
    Done,
    Error(String),
}

pub fn parse_sse_line(line: &str) -> Option<StreamEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') {
        return None;
    }

    let data = trimmed.strip_prefix("data:")?.trim();
    if data.is_empty() {
        return None;
    }

    if data == "[DONE]" {
        return Some(StreamEvent::Done);
    }

    let parsed: Value = match serde_json::from_str(data) {
        Ok(value) => value,
        Err(err) => {
            return Some(StreamEvent::Error(format!(
                "failed to parse stream event JSON: {err}"
            )));
        }
    };

    if let Some(message) = extract_error_message(&parsed) {
        return Some(StreamEvent::Error(message));
    }

    if let Some(text) = extract_text_chunk(&parsed) {
        return Some(StreamEvent::Text(text));
    }

    None
}

fn extract_error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;

    if let Some(message) = error.get("message").and_then(Value::as_str) {
        return Some(message.to_owned());
    }

    if let Some(message) = error.as_str() {
        return Some(message.to_owned());
    }

    Some(error.to_string())
}

fn extract_text_chunk(value: &Value) -> Option<String> {
    if let Some(content) = value
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
    {
        return Some(content.to_owned());
    }

    if let Some(content) = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
    {
        return Some(content.to_owned());
    }

    if let Some(content) = value.pointer("/choices/0/text").and_then(Value::as_str) {
        return Some(content.to_owned());
    }

    if let Some(content) = value.pointer("/delta/text").and_then(Value::as_str) {
        return Some(content.to_owned());
    }

    if let Some(content) = value.pointer("/content/0/text").and_then(Value::as_str) {
        return Some(content.to_owned());
    }

    if let Some(content) = value.pointer("/output_text").and_then(Value::as_str) {
        return Some(content.to_owned());
    }

    None
}
