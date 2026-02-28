use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context as _, Result, anyhow};

#[derive(Debug)]
pub(super) struct IncrementalFileLoader {
    file: File,
    carry_utf8_bytes: Vec<u8>,
    eof: bool,
    bytes_loaded: usize,
    total_bytes: Option<usize>,
}

#[derive(Debug)]
pub(super) struct LoaderChunk {
    pub text: String,
    pub bytes_read: usize,
    pub eof: bool,
}

impl IncrementalFileLoader {
    pub(super) fn open(path: &Path) -> Result<Self> {
        let file =
            File::open(path).with_context(|| format!("failed to read file: {}", path.display()))?;
        let total_bytes = file
            .metadata()
            .ok()
            .and_then(|m| usize::try_from(m.len()).ok());

        Ok(Self {
            file,
            carry_utf8_bytes: Vec::new(),
            eof: false,
            bytes_loaded: 0,
            total_bytes,
        })
    }

    #[inline]
    pub(super) fn bytes_loaded(&self) -> usize {
        self.bytes_loaded
    }

    #[inline]
    pub(super) fn total_bytes(&self) -> Option<usize> {
        self.total_bytes
    }

    #[inline]
    pub(super) fn is_eof(&self) -> bool {
        self.eof
    }

    pub(super) fn read_chunk(&mut self, max_bytes: usize) -> Result<LoaderChunk> {
        if self.eof || max_bytes == 0 {
            return Ok(LoaderChunk {
                text: String::new(),
                bytes_read: 0,
                eof: self.eof,
            });
        }

        let mut buf = vec![0_u8; max_bytes];
        let n = self
            .file
            .read(&mut buf)
            .context("failed while reading file")?;
        buf.truncate(n);
        self.bytes_loaded = self.bytes_loaded.saturating_add(n);
        if n == 0 {
            self.eof = true;
        } else if let Some(total) = self.total_bytes
            && self.bytes_loaded >= total
        {
            self.eof = true;
        }

        let mut joined = Vec::with_capacity(self.carry_utf8_bytes.len() + buf.len());
        joined.extend_from_slice(&self.carry_utf8_bytes);
        joined.extend_from_slice(&buf);
        self.carry_utf8_bytes.clear();

        let text = if self.eof {
            std::str::from_utf8(&joined)
                .map(str::to_owned)
                .map_err(|_| anyhow!("file is not valid UTF-8"))?
        } else {
            match std::str::from_utf8(&joined) {
                Ok(s) => s.to_owned(),
                Err(err) => {
                    if err.error_len().is_some() {
                        return Err(anyhow!("file is not valid UTF-8"));
                    }

                    let valid_up_to = err.valid_up_to();
                    let valid = &joined[..valid_up_to];
                    let tail = &joined[valid_up_to..];
                    self.carry_utf8_bytes.extend_from_slice(tail);

                    std::str::from_utf8(valid)
                        .map(str::to_owned)
                        .map_err(|_| anyhow!("file is not valid UTF-8"))?
                }
            }
        };

        Ok(LoaderChunk {
            text,
            bytes_read: n,
            eof: self.eof,
        })
    }
}
