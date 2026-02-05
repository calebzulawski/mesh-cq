use bitvec::vec::BitVec;
use phf::phf_map;

static MORSE_TABLE: phf::Map<&'static str, &'static str> = phf_map! {
    "A" => ".-",
    "B" => "-...",
    "C" => "-.-.",
    "D" => "-..",
    "E" => ".",
    "F" => "..-.",
    "G" => "--.",
    "H" => "....",
    "I" => "..",
    "J" => ".---",
    "K" => "-.-",
    "L" => ".-..",
    "M" => "--",
    "N" => "-.",
    "O" => "---",
    "P" => ".--.",
    "Q" => "--.-",
    "R" => ".-.",
    "S" => "...",
    "T" => "-",
    "U" => "..-",
    "V" => "...-",
    "W" => ".--",
    "X" => "-..-",
    "Y" => "-.--",
    "Z" => "--..",
    "0" => "-----",
    "1" => ".----",
    "2" => "..---",
    "3" => "...--",
    "4" => "....-",
    "5" => ".....",
    "6" => "-....",
    "7" => "--...",
    "8" => "---..",
    "9" => "----.",
    "." => ".-.-.-",
    "," => "--..--",
    "?" => "..--..",
    "'" => ".----.",
    "!" => "-.-.--",
    "/" => "-..-.",
    "(" => "-.--.",
    ")" => "-.--.-",
    "&" => ".-...",
    ":" => "---...",
    ";" => "-.-.-.",
    "=" => "-...-",
    "+" => ".-.-.",
    "-" => "-....-",
    "_" => "..--.-",
    "\"" => ".-..-.",
    "$" => "...-..-",
    "@" => ".--.-.",
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    UnterminatedProsign(usize),
    UnknownSymbol(String),
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::UnterminatedProsign(pos) => {
                write!(f, "unterminated prosign starting at byte {}", pos)
            }
            EncodeError::UnknownSymbol(sym) => write!(f, "unknown morse symbol: {}", sym),
        }
    }
}

impl std::error::Error for EncodeError {}

/// Encode text into Morse units (1 = tone, 0 = gap).
pub fn encode_units(text: &str) -> Result<BitVec, EncodeError> {
    let mut bits = BitVec::new();
    let mut in_prosign = false;
    let mut last_was_symbol = false;
    let mut prosign_start = 0;

    for (idx, ch) in text.char_indices() {
        if ch == '<' {
            if in_prosign {
                return Err(EncodeError::UnknownSymbol("<".to_string()));
            }
            in_prosign = true;
            prosign_start = idx;
            continue;
        }

        if ch == '>' {
            if !in_prosign {
                return Err(EncodeError::UnknownSymbol(">".to_string()));
            }
            in_prosign = false;
            continue;
        }

        if ch.is_whitespace() {
            if in_prosign {
                continue;
            }
            if last_was_symbol {
                push_units(&mut bits, false, 7);
                last_was_symbol = false;
            }
            continue;
        }

        if last_was_symbol && !in_prosign {
            push_units(&mut bits, false, 3);
        }

        let key = ch.to_ascii_uppercase().to_string();
        let pattern = MORSE_TABLE
            .get(key.as_str())
            .ok_or_else(|| EncodeError::UnknownSymbol(ch.to_string()))?;
        emit_symbol(&mut bits, pattern);
        last_was_symbol = true;
    }

    if in_prosign {
        return Err(EncodeError::UnterminatedProsign(prosign_start));
    }

    Ok(bits)
}

fn emit_symbol(bits: &mut BitVec, pattern: &str) {
    let mut chars = pattern.chars().peekable();
    while let Some(mark) = chars.next() {
        match mark {
            '.' => push_units(bits, true, 1),
            '-' => push_units(bits, true, 3),
            _ => {}
        }

        if chars.peek().is_some() {
            push_units(bits, false, 1);
        }
    }
}

fn push_units(bits: &mut BitVec, value: bool, count: usize) {
    for _ in 0..count {
        bits.push(value);
    }
}
