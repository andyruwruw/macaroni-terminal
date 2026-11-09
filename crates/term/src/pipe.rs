//! Blocking ends for a source that runs on tokio (SSH): output arrives as chunks on a std
//! channel that [`ChunkReader`] reads like a pipe, and input goes out through [`InputWriter`]
//! onto a tokio channel the source's task awaits, along with resizes.

use std::io::{self, Read, Write};
use std::sync::mpsc;

use tokio::sync::mpsc::UnboundedSender;

/// What a session sends its async source.
#[derive(Debug)]
pub(crate) enum Input {
    /// Bytes the user typed (or the emulator's replies).
    Data(Vec<u8>),
    /// The terminal is now this many columns and rows.
    Resize(u16, u16),
}

/// Reads chunks sent on a channel as one stream; EOF once every sender is gone.
pub(crate) struct ChunkReader {
    /// Where chunks arrive.
    chunks: mpsc::Receiver<Vec<u8>>,
    /// The chunk being read, and how far.
    current: Vec<u8>,
    /// Bytes of `current` already read.
    at: usize,
}

impl ChunkReader {
    /// A reader of what `chunks` delivers.
    pub(crate) fn new(chunks: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            chunks,
            current: Vec::new(),
            at: 0,
        }
    }
}

impl Read for ChunkReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.at >= self.current.len() {
            match self.chunks.recv() {
                Ok(chunk) => {
                    self.current = chunk;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.current.len() - self.at);
        buf[..n].copy_from_slice(&self.current[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// Sends what's written as [`Input::Data`].
pub(crate) struct InputWriter(pub(crate) UnboundedSender<Input>);

impl Write for InputWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .send(Input::Data(buf.to_vec()))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_chunks_as_a_stream() {
        let (tx, rx) = mpsc::channel();
        tx.send(b"hello ".to_vec()).unwrap();
        tx.send(b"world".to_vec()).unwrap();
        drop(tx);
        let mut reader = ChunkReader::new(rx);
        let mut out = String::new();
        reader.read_to_string(&mut out).unwrap();
        assert_eq!(out, "hello world");
    }
}
