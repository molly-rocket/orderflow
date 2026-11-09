use std::io::{self, BufRead, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub recv_us: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Fixture {
    frames: Vec<Frame>,
}

#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    #[error("fixture I/O: {0}")]
    Io(#[from] io::Error),
    #[error("fixture line {line}: {source}")]
    Line {
        line: usize,
        source: serde_json::Error,
    },
}

impl Fixture {
    pub fn new(frames: Vec<Frame>) -> Self {
        Self { frames }
    }

    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, FixtureError> {
        let file = std::fs::File::open(path)?;
        Self::read_jsonl(io::BufReader::new(file))
    }

    pub fn parse_jsonl(text: &str) -> Result<Self, FixtureError> {
        Self::read_jsonl(text.as_bytes())
    }

    pub fn read_jsonl(reader: impl BufRead) -> Result<Self, FixtureError> {
        let mut frames = Vec::new();
        for (index, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let frame = serde_json::from_str(&line).map_err(|source| FixtureError::Line {
                line: index + 1,
                source,
            })?;
            frames.push(frame);
        }
        Ok(Self { frames })
    }

    pub fn write_jsonl(&self, mut writer: impl Write) -> io::Result<()> {
        for frame in &self.frames {
            serde_json::to_writer(&mut writer, frame)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()
    }
}

impl FromIterator<Frame> for Fixture {
    fn from_iter<I: IntoIterator<Item = Frame>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}
