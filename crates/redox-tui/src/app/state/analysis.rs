use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use redox_core::{BufferId, TextBuffer};

use crate::ui::overlays::{DelimiterAnalysis, compute_delimiter_analysis};
use crate::ui::syntax::{HighlightCache, SyntaxHighlighter, SyntaxLanguage};

#[derive(Debug)]
pub(super) enum AnalysisResult {
    Syntax {
        buffer_id: BufferId,
        version: u64,
        syntax_cache: Option<HighlightCache>,
    },
    Delimiters {
        buffer_id: BufferId,
        version: u64,
        delimiter_analysis: DelimiterAnalysis,
    },
}

struct AnalysisRequest {
    buffer_id: BufferId,
    version: u64,
    buffer: TextBuffer,
    syntax_language: Option<SyntaxLanguage>,
}

pub(super) struct AnalysisWorker {
    requests: LatestRequestSender,
    results: Receiver<AnalysisResult>,
}

#[derive(Default)]
struct LatestRequestSlot {
    request: Option<AnalysisRequest>,
    closed: bool,
}

struct LatestRequestSender {
    state: Arc<(Mutex<LatestRequestSlot>, Condvar)>,
}

struct LatestRequestReceiver {
    state: Arc<(Mutex<LatestRequestSlot>, Condvar)>,
}

impl std::fmt::Debug for AnalysisWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisWorker").finish_non_exhaustive()
    }
}

impl AnalysisWorker {
    pub(super) fn new() -> Self {
        let (request_tx, request_rx) = latest_request_channel();
        let (result_tx, result_rx) = mpsc::channel::<AnalysisResult>();

        thread::Builder::new()
            .name("redox-analysis".to_string())
            .spawn(move || {
                while let Some(mut request) = request_rx.recv() {
                    loop {
                        request = drain_latest_requests(request, &request_rx);
                        let syntax_cache = request.syntax_language.and_then(|language| {
                            SyntaxHighlighter::compute_cache(&request.buffer, language)
                        });
                        if result_tx
                            .send(AnalysisResult::Syntax {
                                buffer_id: request.buffer_id,
                                version: request.version,
                                syntax_cache,
                            })
                            .is_err()
                        {
                            return;
                        }

                        if let Some(next_request) = request_rx.try_recv() {
                            request = next_request;
                            continue;
                        }

                        let delimiter_analysis = compute_delimiter_analysis(&request.buffer);
                        if result_tx
                            .send(AnalysisResult::Delimiters {
                                buffer_id: request.buffer_id,
                                version: request.version,
                                delimiter_analysis,
                            })
                            .is_err()
                        {
                            return;
                        }

                        break;
                    }
                }
            })
            .expect("failed to start analysis worker");

        Self {
            requests: request_tx,
            results: result_rx,
        }
    }

    pub(super) fn request(
        &self,
        buffer_id: BufferId,
        version: u64,
        buffer: TextBuffer,
        syntax_language: Option<SyntaxLanguage>,
    ) {
        self.requests.send_latest(AnalysisRequest {
            buffer_id,
            version,
            buffer,
            syntax_language,
        });
    }

    pub(super) fn try_recv(&self) -> Option<AnalysisResult> {
        self.results.try_recv().ok()
    }
}

impl LatestRequestSender {
    fn send_latest(&self, request: AnalysisRequest) {
        let (lock, available) = &*self.state;
        let mut slot = lock.lock().expect("analysis request lock poisoned");
        if slot.closed {
            return;
        }
        slot.request = Some(request);
        available.notify_one();
    }
}

impl Drop for LatestRequestSender {
    fn drop(&mut self) {
        let (lock, available) = &*self.state;
        let mut slot = lock.lock().expect("analysis request lock poisoned");
        slot.closed = true;
        available.notify_one();
    }
}

impl LatestRequestReceiver {
    fn recv(&self) -> Option<AnalysisRequest> {
        let (lock, available) = &*self.state;
        let mut slot = lock.lock().expect("analysis request lock poisoned");
        loop {
            if let Some(request) = slot.request.take() {
                return Some(request);
            }
            if slot.closed {
                return None;
            }
            slot = available
                .wait(slot)
                .expect("analysis request lock poisoned");
        }
    }

    fn try_recv(&self) -> Option<AnalysisRequest> {
        let (lock, _) = &*self.state;
        let mut slot = lock.lock().expect("analysis request lock poisoned");
        slot.request.take()
    }
}

fn latest_request_channel() -> (LatestRequestSender, LatestRequestReceiver) {
    let state = Arc::new((Mutex::new(LatestRequestSlot::default()), Condvar::new()));
    (
        LatestRequestSender {
            state: Arc::clone(&state),
        },
        LatestRequestReceiver { state },
    )
}

fn drain_latest_requests(
    first: AnalysisRequest,
    receiver: &LatestRequestReceiver,
) -> AnalysisRequest {
    let mut latest = first;
    while let Some(request) = receiver.try_recv() {
        latest = request;
    }
    latest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(buffer_id: BufferId, version: u64) -> AnalysisRequest {
        AnalysisRequest {
            buffer_id,
            version,
            buffer: TextBuffer::from_text("fn main() {}\n"),
            syntax_language: Some(SyntaxLanguage::Rust),
        }
    }

    #[test]
    fn latest_request_channel_replaces_pending_request() {
        let _lock = crate::app::state::global_test_state_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (sender, receiver) = latest_request_channel();
        let buffer_id = redox_core::EditorSession::open_initial_unnamed()
            .expect("session")
            .active_id();
        sender.send_latest(request(buffer_id, 1));
        sender.send_latest(request(buffer_id, 2));

        let received = receiver.recv().expect("latest request");

        assert_eq!(received.version, 2);
        assert!(receiver.try_recv().is_none());
    }

    #[test]
    fn drain_latest_requests_returns_newest_pending_request() {
        let _lock = crate::app::state::global_test_state_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (sender, receiver) = latest_request_channel();
        let buffer_id = redox_core::EditorSession::open_initial_unnamed()
            .expect("session")
            .active_id();
        sender.send_latest(request(buffer_id, 2));
        sender.send_latest(request(buffer_id, 3));

        let drained = drain_latest_requests(request(buffer_id, 1), &receiver);

        assert_eq!(drained.version, 3);
        assert!(receiver.try_recv().is_none());
    }
}
