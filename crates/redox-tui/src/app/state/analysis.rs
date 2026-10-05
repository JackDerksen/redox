use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use redox_core::{BufferId, TextBuffer};

use crate::ui::overlays::{DelimiterAnalysis, compute_delimiter_analysis};
use crate::ui::syntax::{HighlightCache, SyntaxLanguage, SyntaxParser};

#[derive(Debug)]
pub(super) enum AnalysisResult {
    Syntax {
        request_id: u64,
        buffer_id: BufferId,
        version: u64,
        syntax_cache: Option<HighlightCache>,
    },
    Delimiters {
        request_id: u64,
        buffer_id: BufferId,
        version: u64,
        delimiter_analysis: DelimiterAnalysis,
    },
}

struct AnalysisRequest {
    request_id: u64,
    buffer_id: BufferId,
    version: u64,
    buffer: TextBuffer,
    syntax_language: Option<SyntaxLanguage>,
}

pub(super) struct AnalysisWorker {
    requests: LatestRequestSender,
    results: Receiver<AnalysisResult>,
    last_request_id: Cell<u64>,
    pending: RefCell<HashMap<BufferId, (u64, u64)>>,
}

#[derive(Default)]
struct LatestRequestQueue {
    requests: VecDeque<AnalysisRequest>,
    closed: bool,
}

struct LatestRequestSender {
    state: Arc<(Mutex<LatestRequestQueue>, Condvar)>,
}

struct LatestRequestReceiver {
    state: Arc<(Mutex<LatestRequestQueue>, Condvar)>,
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
                let mut syntax_parser = SyntaxParser::default();
                while let Some(request) = request_rx.recv() {
                    // Publish overlays before the more expensive syntax queries.
                    let delimiter_analysis = compute_delimiter_analysis(&request.buffer);
                    if result_tx
                        .send(AnalysisResult::Delimiters {
                            request_id: request.request_id,
                            buffer_id: request.buffer_id,
                            version: request.version,
                            delimiter_analysis,
                        })
                        .is_err()
                    {
                        return;
                    }
                    let syntax_cache = request.syntax_language.and_then(|language| {
                        syntax_parser.compute_cache(&request.buffer, language)
                    });
                    if result_tx
                        .send(AnalysisResult::Syntax {
                            request_id: request.request_id,
                            buffer_id: request.buffer_id,
                            version: request.version,
                            syntax_cache,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            })
            .expect("failed to start analysis worker");

        Self {
            requests: request_tx,
            results: result_rx,
            last_request_id: Cell::new(0),
            pending: RefCell::default(),
        }
    }

    pub(super) fn request(
        &self,
        buffer_id: BufferId,
        version: u64,
        buffer: TextBuffer,
        syntax_language: Option<SyntaxLanguage>,
    ) {
        let request_id = self
            .last_request_id
            .get()
            .checked_add(1)
            .expect("analysis request ID overflow");
        self.last_request_id.set(request_id);
        self.pending
            .borrow_mut()
            .insert(buffer_id, (request_id, version));
        self.requests.send_latest(AnalysisRequest {
            request_id,
            buffer_id,
            version,
            buffer,
            syntax_language,
        });
    }

    pub(super) fn try_recv(&self) -> Option<AnalysisResult> {
        loop {
            let result = self.results.try_recv().ok()?;
            let (request_id, buffer_id, version) = match &result {
                AnalysisResult::Syntax {
                    request_id,
                    buffer_id,
                    version,
                    ..
                }
                | AnalysisResult::Delimiters {
                    request_id,
                    buffer_id,
                    version,
                    ..
                } => (*request_id, *buffer_id, *version),
            };
            let mut pending = self.pending.borrow_mut();
            if pending.get(&buffer_id) != Some(&(request_id, version)) {
                continue;
            }
            if matches!(result, AnalysisResult::Syntax { .. }) {
                pending.remove(&buffer_id);
            }
            return Some(result);
        }
    }
}

impl AnalysisWorker {
    pub(super) fn is_pending(&self) -> bool {
        !self.pending.borrow().is_empty()
    }
}

impl LatestRequestSender {
    fn send_latest(&self, request: AnalysisRequest) {
        let (lock, available) = &*self.state;
        let mut queue = lock.lock().expect("analysis request lock poisoned");
        if queue.closed {
            return;
        }
        // Coalesce edits to one buffer without dropping work for other buffers.
        if let Some(pending) = queue
            .requests
            .iter_mut()
            .find(|pending| pending.buffer_id == request.buffer_id)
        {
            *pending = request;
        } else {
            queue.requests.push_back(request);
        }
        available.notify_one();
    }
}

impl Drop for LatestRequestSender {
    fn drop(&mut self) {
        let (lock, available) = &*self.state;
        let mut queue = lock.lock().expect("analysis request lock poisoned");
        queue.closed = true;
        available.notify_one();
    }
}

impl LatestRequestReceiver {
    fn recv(&self) -> Option<AnalysisRequest> {
        let (lock, available) = &*self.state;
        let mut queue = lock.lock().expect("analysis request lock poisoned");
        loop {
            if let Some(request) = queue.requests.pop_front() {
                return Some(request);
            }
            if queue.closed {
                return None;
            }
            queue = available
                .wait(queue)
                .expect("analysis request lock poisoned");
        }
    }
}

fn latest_request_channel() -> (LatestRequestSender, LatestRequestReceiver) {
    let state = Arc::new((Mutex::new(LatestRequestQueue::default()), Condvar::new()));
    (
        LatestRequestSender {
            state: Arc::clone(&state),
        },
        LatestRequestReceiver { state },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(buffer_id: BufferId, version: u64) -> AnalysisRequest {
        AnalysisRequest {
            request_id: version,
            buffer_id,
            version,
            buffer: TextBuffer::from_text("fn main() {}\n"),
            syntax_language: Some(SyntaxLanguage::Rust),
        }
    }

    #[test]
    fn latest_request_channel_coalesces_each_buffer_independently() {
        let _lock = crate::app::state::global_test_state_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (sender, receiver) = latest_request_channel();
        let mut session = redox_core::EditorSession::open_initial_unnamed().expect("session");
        let first_buffer = session.active_id();
        let second_buffer = session.open_unnamed_buffer();
        sender.send_latest(request(first_buffer, 1));
        sender.send_latest(request(second_buffer, 1));
        sender.send_latest(request(first_buffer, 2));
        drop(sender);

        let received = receiver.recv().expect("latest request");

        assert_eq!(received.buffer_id, first_buffer);
        assert_eq!(received.version, 2);
        let received = receiver.recv().expect("other buffer request");
        assert_eq!(received.buffer_id, second_buffer);
        assert_eq!(received.version, 1);
        assert!(receiver.recv().is_none());
    }
}
