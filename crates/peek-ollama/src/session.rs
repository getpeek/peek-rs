//! The tokio runtime every Ollama call runs on.

use tokio::runtime::Runtime;
use tokio::sync::{mpsc, oneshot};

use crate::chat::{Chat, ChatDelta, ToolCall};
use crate::wire::{ChatChunk, ChatRequest, Options};

/// The reference's settings, carried over so a model behaves the same in both apps.
const KEEP_ALIVE: &str = "10m";
const NUM_THREAD: u32 = 32;

pub type Pending<T> = oneshot::Receiver<Result<T, OllamaError>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OllamaError {
    /// The server could not be reached, or answered with a status.
    Request(String),
    /// The stream ended in a way that could not be read.
    Stream(String),
}

impl std::fmt::Display for OllamaError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(message) | Self::Stream(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for OllamaError {}

/// A turn in flight: the deltas as they arrive, and the handle that cancels it.
///
/// Dropping `Turn` aborts the request — which the reference cannot do, because its abort is a
/// polled boolean and the HTTP request runs on to completion regardless.
#[derive(Debug)]
pub struct Turn {
    deltas: mpsc::UnboundedReceiver<ChatDelta>,
    _abort: AbortOnDrop,
}

impl Turn {
    /// The next piece of the turn, or `None` once it has ended.
    pub async fn next(&mut self) -> Option<ChatDelta> {
        self.deltas.recv().await
    }
}

#[derive(Debug)]
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct OllamaSession {
    client: reqwest::Client,
    url: String,
    model: String,
    runtime: Runtime,
}

impl std::fmt::Debug for OllamaSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OllamaSession")
            .field("url", &self.url)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl OllamaSession {
    /// Builds the runtime and the client. No request is made until a turn starts.
    ///
    /// # Errors
    /// Returns an error if the tokio runtime cannot be built.
    pub fn new(url: &str, model: &str) -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("peek-ollama")
            .enable_all()
            .build()?;
        Ok(Self {
            client: reqwest::Client::new(),
            // A trailing slash would make the endpoint `//api/chat`, which some proxies reject.
            url: url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            runtime,
        })
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Starts a turn. Every token arrives on the returned [`Turn`]; dropping it cancels.
    #[must_use]
    pub fn chat(&self, chat: Chat) -> Turn {
        let (sender, deltas) = mpsc::unbounded_channel();
        let request = ChatRequest {
            model: self.model.clone(),
            messages: chat.messages,
            tools: chat.tools,
            stream: true,
            keep_alive: KEEP_ALIVE,
            think: false,
            options: Options {
                num_thread: NUM_THREAD,
            },
        };
        let client = self.client.clone();
        let endpoint = format!("{}/api/chat", self.url);

        let handle = self.runtime.spawn(async move {
            let outcome = stream(&client, &endpoint, &request, &sender).await;
            // The receiver going away means the caller stopped caring, which is not an error.
            let _ = match outcome {
                Ok(answer) => sender.send(ChatDelta::Done(answer)),
                Err(error) => sender.send(ChatDelta::Failed(error.to_string())),
            };
        });
        Turn {
            deltas,
            _abort: AbortOnDrop(handle),
        }
    }
}

/// Reads the NDJSON body a line at a time, forwarding each token as it lands.
async fn stream(
    client: &reqwest::Client,
    endpoint: &str,
    request: &ChatRequest,
    sender: &mpsc::UnboundedSender<ChatDelta>,
) -> Result<String, OllamaError> {
    let response = client
        .post(endpoint)
        .json(request)
        .send()
        .await
        .map_err(|error| OllamaError::Request(error.to_string()))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(OllamaError::Request(format!(
            "the model server answered {status}: {}",
            body.trim()
        )));
    }

    let mut response = response;
    let mut answer = String::new();
    let mut calls: Vec<ToolCall> = Vec::new();
    // A chunk boundary can split a line, so whatever is left over is carried forward.
    let mut buffer = String::new();

    while let Some(bytes) = response
        .chunk()
        .await
        .map_err(|error| OllamaError::Stream(error.to_string()))?
    {
        buffer.push_str(&String::from_utf8_lossy(&bytes));
        while let Some(newline) = buffer.find('\n') {
            let line: String = buffer.drain(..=newline).collect();
            read_line(line.trim(), (&mut answer, &mut calls), sender);
        }
    }
    // A server that ends without a trailing newline still owes us its last object.
    read_line(buffer.trim(), (&mut answer, &mut calls), sender);

    if !calls.is_empty() {
        let _ = sender.send(ChatDelta::Calls(calls));
    }
    Ok(answer)
}

/// One NDJSON object. A line that will not parse is skipped rather than failing the turn: a
/// proxy injecting a keepalive should not lose the answer.
fn read_line(
    line: &str,
    state: (&mut String, &mut Vec<ToolCall>),
    sender: &mpsc::UnboundedSender<ChatDelta>,
) {
    let (answer, calls) = state;
    if line.is_empty() {
        return;
    }
    let Ok(chunk) = serde_json::from_str::<ChatChunk>(line) else {
        log::debug!("peek: skipping an unreadable line from the model server");
        return;
    };
    let Some(message) = chunk.message else {
        return;
    };

    if !message.content.is_empty() {
        answer.push_str(&message.content);
        let _ = sender.send(ChatDelta::Chunk(message.content));
    }
    for call in message.tool_calls.unwrap_or_default() {
        calls.push(ToolCall {
            id: format!("call_{}", calls.len() + 1),
            name: call.function.name,
            args: call.function.arguments,
        });
    }
    let _ = chunk.done;
}

#[cfg(test)]
mod tests {
    use super::{ChatDelta, read_line};
    use crate::chat::ToolCall;
    use tokio::sync::mpsc;

    fn read(lines: &[&str]) -> (String, Vec<ToolCall>, Vec<ChatDelta>) {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut answer = String::new();
        let mut calls = Vec::new();
        for line in lines {
            read_line(line, (&mut answer, &mut calls), &sender);
        }
        drop(sender);

        let mut deltas = Vec::new();
        while let Ok(delta) = receiver.try_recv() {
            deltas.push(delta);
        }
        (answer, calls, deltas)
    }

    #[test]
    fn tokens_accumulate_and_are_forwarded_as_they_land() {
        let (answer, _, deltas) = read(&[
            r#"{"message":{"content":"Hello "}}"#,
            r#"{"message":{"content":"world"},"done":true}"#,
        ]);

        assert_eq!(answer, "Hello world");
        assert_eq!(
            deltas,
            vec![
                ChatDelta::Chunk("Hello ".to_string()),
                ChatDelta::Chunk("world".to_string()),
            ]
        );
    }

    /// Arguments arrive already parsed — reading them as a string would double-encode them on
    /// the way back into the next turn.
    #[test]
    fn tool_calls_are_collected_with_their_arguments_as_json() {
        let (_, calls, _) = read(&[
            r#"{"message":{"content":"","tool_calls":[{"function":{"name":"get_pages","arguments":{"pageId":"page_1"}}}]}}"#,
        ]);

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_pages");
        assert_eq!(calls[0].args["pageId"], serde_json::json!("page_1"));
        assert_eq!(calls[0].id, "call_1", "ids are minted, not returned");
    }

    /// A proxy that injects a keepalive, or a server that writes a blank line, must not cost
    /// the turn its answer.
    #[test]
    fn an_unreadable_line_is_skipped_rather_than_failing_the_turn() {
        let (answer, _, _) = read(&[
            r#"{"message":{"content":"before "}}"#,
            "",
            "not json at all",
            r#"{"message":{"content":"after"},"done":true}"#,
        ]);
        assert_eq!(answer, "before after");
    }

    #[test]
    fn a_line_with_no_message_is_ignored() {
        let (answer, calls, deltas) = read(&[r#"{"done":true}"#]);
        assert!(answer.is_empty());
        assert!(calls.is_empty());
        assert!(deltas.is_empty());
    }
}
