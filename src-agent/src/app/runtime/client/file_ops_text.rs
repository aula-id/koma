//! Lossless text formats accepted by the Coding editor. Monaco/LSP use LF text;
//! encoding, BOM and uniform line endings are restored at the filesystem edge.

#[derive(Clone, Copy, Default)]
enum Encoding {
    #[default]
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum LineEnding {
    #[default]
    Lf,
    CrLf,
    Cr,
}

#[derive(Clone, Copy, Default)]
pub(super) struct TextFormat {
    encoding: Encoding,
    line_ending: LineEnding,
}

pub(super) enum DecodeError {
    Binary,
    UnsupportedEncoding,
    MixedLineEndings,
}

impl DecodeError {
    pub(super) fn message(&self) -> &'static str {
        match self {
            Self::Binary => "Binary file — no editable text preview",
            Self::UnsupportedEncoding => "Unsupported text encoding. The editor accepts UTF-8 and UTF-16 with a BOM; the original file has not been converted.",
            Self::MixedLineEndings => "This file mixes line endings. Editing is unavailable to avoid changing its format; normalize line endings explicitly before editing.",
        }
    }
}

pub(super) struct TextFile {
    pub content: String,
    pub format: TextFormat,
}

pub(super) fn decode(bytes: &[u8]) -> Result<TextFile, DecodeError> {
    // Check UTF-32 before UTF-16LE, since their BOMs share the same prefix.
    if bytes.starts_with(&[0xff, 0xfe, 0, 0]) || bytes.starts_with(&[0, 0, 0xfe, 0xff]) {
        return Err(DecodeError::UnsupportedEncoding);
    }
    let (text, encoding) = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let little = bytes[0] == 0xff;
        let bytes = &bytes[2..];
        if bytes.len() % 2 != 0 {
            return Err(DecodeError::UnsupportedEncoding);
        }
        let units: Vec<_> = bytes
            .chunks_exact(2)
            .map(|c| {
                if little {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        let text = String::from_utf16(&units).map_err(|_| DecodeError::UnsupportedEncoding)?;
        (
            text,
            if little {
                Encoding::Utf16Le
            } else {
                Encoding::Utf16Be
            },
        )
    } else {
        // Keep binary detection independent of Unicode errors for media viewers.
        if bytes.contains(&0) {
            return Err(DecodeError::Binary);
        }
        let (bytes, encoding) = if let Some(body) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
            (body, Encoding::Utf8Bom)
        } else {
            (bytes, Encoding::Utf8)
        };
        let text = std::str::from_utf8(bytes).map_err(|_| DecodeError::UnsupportedEncoding)?;
        (text.to_owned(), encoding)
    };
    if text.contains('\0') {
        return Err(DecodeError::Binary);
    }

    let mut line_ending = None;
    let mut bytes = text.bytes().peekable();
    while let Some(b) = bytes.next() {
        let ending = match b {
            b'\r' if bytes.peek() == Some(&b'\n') => {
                bytes.next();
                LineEnding::CrLf
            }
            b'\r' => LineEnding::Cr,
            b'\n' => LineEnding::Lf,
            _ => continue,
        };
        if line_ending.is_some_and(|previous| previous != ending) {
            return Err(DecodeError::MixedLineEndings);
        }
        line_ending = Some(ending);
    }
    Ok(TextFile {
        content: text.replace("\r\n", "\n").replace('\r', "\n"),
        format: TextFormat {
            encoding,
            line_ending: line_ending.unwrap_or_default(),
        },
    })
}

impl TextFormat {
    pub(super) fn encode(self, content: &str) -> Vec<u8> {
        let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
        let text = match self.line_ending {
            LineEnding::Lf => normalized,
            LineEnding::CrLf => normalized.replace('\n', "\r\n"),
            LineEnding::Cr => normalized.replace('\n', "\r"),
        };
        match self.encoding {
            Encoding::Utf8 => text.into_bytes(),
            Encoding::Utf8Bom => {
                let mut bytes = vec![0xef, 0xbb, 0xbf];
                bytes.extend_from_slice(text.as_bytes());
                bytes
            }
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let little = matches!(self.encoding, Encoding::Utf16Le);
                let mut bytes = if little {
                    vec![0xff, 0xfe]
                } else {
                    vec![0xfe, 0xff]
                };
                for unit in text.encode_utf16() {
                    bytes.extend_from_slice(&if little {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    });
                }
                bytes
            }
        }
    }
}
