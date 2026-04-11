use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use redox_core::{BufferId, TextBuffer};

use crate::ui::overlays::{DelimiterAnalysis, compute_delimiter_analysis};
use crate::ui::syntax::{HighlightCache, SyntaxHighlighter, SyntaxLanguage};

#[derive(Debug)]
pub(super) struct AnalysisResult {
    pub(super) buffer_id: BufferId,
    pub(super) version: u64,
    pub(super) syntax_cache: Option<HighlightCache>,
    pub(super) delimiter_analysis: DelimiterAnalysis,
}

struct AnalysisRequest {
    buffer_id: BufferId,
    version: u64,
    buffer: TextBuffer,
    syntax_language: Option<SyntaxLanguage>,
}

pub(super) struct AnalysisWorker {
    requests: Sender<AnalysisRequest>,
    results: Receiver<AnalysisResult>,
}

impl std::fmt::Debug for AnalysisWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisWorker").finish_non_exhaustive()
    }
}

impl AnalysisWorker {
    pub(super) fn new() -> Self {
        let (request_tx, request_rx) = mpsc::channel::<AnalysisRequest>();
        let (result_tx, result_rx) = mpsc::channel::<AnalysisResult>();

        thread::Builder::new()
            .name("redox-analysis".to_string())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let syntax_cache = request.syntax_language.and_then(|language| {
                        SyntaxHighlighter::compute_cache(&request.buffer, language)
                    });
                    let delimiter_analysis = compute_delimiter_analysis(&request.buffer);

                    if result_tx
                        .send(AnalysisResult {
                            buffer_id: request.buffer_id,
                            version: request.version,
                            syntax_cache,
                            delimiter_analysis,
                        })
                        .is_err()
                    {
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
        let _ = self.requests.send(AnalysisRequest {
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
