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

        // Read straight after any bytes of a char split by the previous chunk.
        let mut bytes = std::mem::take(&mut self.carry_utf8_bytes);
        let carried = bytes.len();
        bytes.resize(carried + max_bytes, 0);
        let n = self
            .file
            .read(&mut bytes[carried..])
            .context("failed while reading file")?;
        bytes.truncate(carried + n);
        self.bytes_loaded = self.bytes_loaded.saturating_add(n);
        if n == 0 {
            self.eof = true;
        } else if let Some(total) = self.total_bytes
            && self.bytes_loaded >= total
        {
            self.eof = true;
        }

        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(err) if self.eof || err.utf8_error().error_len().is_some() => {
                return Err(anyhow!("file is not valid UTF-8"));
            }
            Err(err) => {
                // The chunk ends partway through a char; keep it for the next read.
                let valid_up_to = err.utf8_error().valid_up_to();
                let mut bytes = err.into_bytes();
                self.carry_utf8_bytes = bytes.split_off(valid_up_to);
                String::from_utf8(bytes).map_err(|_| anyhow!("file is not valid UTF-8"))?
            }
        };

        Ok(LoaderChunk {
            text,
            bytes_read: n,
            eof: self.eof,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::IncrementalFileLoader;

    fn read_in_chunks(bytes: &[u8], chunk: usize) -> anyhow::Result<String> {
        let path = std::env::temp_dir().join(format!(
            "redox_loader_{}_{chunk}_{}",
            std::process::id(),
            bytes.len()
        ));
        std::fs::write(&path, bytes)?;
        let mut loader = IncrementalFileLoader::open(&path)?;
        let mut text = String::new();
        let result = loop {
            match loader.read_chunk(chunk) {
                Ok(read) => {
                    text.push_str(&read.text);
                    if read.eof {
                        break Ok(text);
                    }
                }
                Err(error) => break Err(error),
            }
        };
        let _ = std::fs::remove_file(path);
        result
    }

    #[test]
    fn chunks_split_inside_chars_join_back_together() {
        let text = "aé漢🦀b\nsecond line é\n".repeat(3);
        for chunk in 1..=7 {
            assert_eq!(read_in_chunks(text.as_bytes(), chunk).unwrap(), text);
        }
        assert!(read_in_chunks(b"ok \xff bad", 3).is_err());
        assert!(read_in_chunks("cut \u{e9}".as_bytes().split_last().unwrap().1, 2).is_err());
    }
}
