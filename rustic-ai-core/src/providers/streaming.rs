use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum StreamEvent {
    Text(String),
    Done,
    Error(String),
}

pub fn spawn_sse_stream(response: reqwest::Response) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        let mut stream = response.bytes_stream();
        let mut line_buffer = String::new();

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(bytes) => bytes,
                Err(err) => {
                    let _ = tx
                        .send(format!("[stream error] failed to read stream chunk: {err}"))
                        .await;
                    break;
                }
            };

            let decoded = match std::str::from_utf8(&chunk) {
                Ok(text) => text,
                Err(err) => {
                    let _ = tx
                        .send(format!(
                            "[stream error] invalid UTF-8 in stream chunk: {err}"
                        ))
                        .await;
                    break;
                }
            };

            line_buffer.push_str(decoded);

            while let Some(idx) = line_buffer.find('\n') {
                let line = line_buffer[..idx].trim_end_matches('\r').to_owned();
                line_buffer.drain(..=idx);

                match parse_sse_line(&line) {
                    Some(StreamEvent::Text(text)) => {
                        if tx.send(text).await.is_err() {
                            return;
                        }
                    }
                    Some(StreamEvent::Error(err)) => {
                        let _ = tx.send(format!("[stream error] {err}")).await;
                        return;
                    }
                    Some(StreamEvent::Done) => return,
                    None => {}
                }
            }
        }

        if !line_buffer.is_empty() {
            match parse_sse_line(line_buffer.trim_end_matches('\r')) {
                Some(StreamEvent::Text(text)) => {
                    let _ = tx.send(text).await;
                }
                Some(StreamEvent::Error(err)) => {
                    let _ = tx.send(format!("[stream error] {err}")).await;
                }
                Some(StreamEvent::Done) | None => {}
            }
        }
    });

    rx
}

pub fn spawn_sse_stream_with_data_parser(
    response: reqwest::Response,
    parse_data: fn(&str) -> Option<String>,
) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        let mut stream = response.bytes_stream();
        let mut line_buffer = String::new();

        while let Some(chunk_result) = stream.next().await {
            let chunk = match chunk_result {
                Ok(bytes) => bytes,
                Err(err) => {
                    let _ = tx
                        .send(format!("[stream error] failed to read stream chunk: {err}"))
                        .await;
                    break;
                }
            };

            let decoded = match std::str::from_utf8(&chunk) {
                Ok(text) => text,
                Err(err) => {
                    let _ = tx
                        .send(format!(
                            "[stream error] invalid UTF-8 in stream chunk: {err}"
                        ))
                        .await;
                    break;
                }
            };

            line_buffer.push_str(decoded);

            while let Some(idx) = line_buffer.find('\n') {
                let line = line_buffer[..idx].trim_end_matches('\r').to_owned();
                line_buffer.drain(..=idx);
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with(':') {
                    continue;
                }

                let Some(data) = trimmed.strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" {
                    return;
                }

                if let Some(text) = parse_data(data) {
                    if tx.send(text).await.is_err() {
                        return;
                    }
                }
            }
        }

        if !line_buffer.is_empty() {
            let trimmed = line_buffer.trim_end_matches('\r').trim();
            if let Some(data) = trimmed.strip_prefix("data:") {
                let data = data.trim();
                if data != "[DONE]" {
                    if let Some(text) = parse_data(data) {
                        let _ = tx.send(text).await;
                    }
                }
            }
        }
    });

    rx
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
