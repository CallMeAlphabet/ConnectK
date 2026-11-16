use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn int(value: i64) -> Json {
        Json::Number(value.to_string())
    }

    pub fn uint(value: u64) -> Json {
        Json::Number(value.to_string())
    }

    pub fn object(fields: Vec<(&str, Json)>) -> Json {
        Json::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(name, _)| name == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Number(raw) => raw
                .parse::<i64>()
                .ok()
                .or_else(|| raw.parse::<f64>().ok().map(|value| value as i64)),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        self.as_i64().and_then(|value| i32::try_from(value).ok())
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    pub fn encode(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Number(raw) => out.push_str(raw),
            Json::String(value) => write_string(value, out),
            Json::Array(values) => {
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    value.write(out);
                }
                out.push(']');
            }
            Json::Object(fields) => {
                out.push('{');
                for (index, (key, value)) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write_string(key, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }

    pub fn parse(text: &str) -> Result<Json, String> {
        let mut parser = Parser {
            chars: text.chars().collect(),
            pos: 0,
        };
        let value = parser.value(0)?;
        parser.whitespace();
        if parser.pos != parser.chars.len() {
            return Err(format!("trailing data at character {}", parser.pos));
        }
        Ok(value)
    }
}

impl From<bool> for Json {
    fn from(value: bool) -> Json {
        Json::Bool(value)
    }
}

impl From<i32> for Json {
    fn from(value: i32) -> Json {
        Json::int(value as i64)
    }
}

impl From<i64> for Json {
    fn from(value: i64) -> Json {
        Json::int(value)
    }
}

impl From<u64> for Json {
    fn from(value: u64) -> Json {
        Json::uint(value)
    }
}

impl From<usize> for Json {
    fn from(value: usize) -> Json {
        Json::uint(value as u64)
    }
}

impl From<&str> for Json {
    fn from(value: &str) -> Json {
        Json::String(value.to_string())
    }
}

impl From<String> for Json {
    fn from(value: String) -> Json {
        Json::String(value)
    }
}

impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(value: Option<T>) -> Json {
        value.map(Into::into).unwrap_or(Json::Null)
    }
}

fn write_string(value: &str, out: &mut String) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            ch if ch < ' ' => {
                let _ = write!(out, "\\u{:04x}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > 64 {
            return Err("JSON is nested too deeply".into());
        }
        self.whitespace();
        match self.peek() {
            Some('{') => self.object(depth),
            Some('[') => self.array(depth),
            Some('"') => Ok(Json::String(self.string()?)),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('n') => self.literal("null", Json::Null),
            Some('-') | Some('0'..='9') => self.number(),
            Some(ch) => Err(format!("unexpected character '{ch}' at {}", self.pos)),
            None => Err("unexpected end of JSON".into()),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        self.expect('{')?;
        let mut fields = Vec::new();
        self.whitespace();
        if self.consume('}') {
            return Ok(Json::Object(fields));
        }
        loop {
            self.whitespace();
            let key = self.string()?;
            self.whitespace();
            self.expect(':')?;
            let value = self.value(depth + 1)?;
            fields.push((key, value));
            self.whitespace();
            if self.consume('}') {
                return Ok(Json::Object(fields));
            }
            self.expect(',')?;
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, String> {
        self.expect('[')?;
        let mut values = Vec::new();
        self.whitespace();
        if self.consume(']') {
            return Ok(Json::Array(values));
        }
        loop {
            values.push(self.value(depth + 1)?);
            self.whitespace();
            if self.consume(']') {
                return Ok(Json::Array(values));
            }
            self.expect(',')?;
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            let Some(ch) = self.next() else {
                return Err("unterminated JSON string".into());
            };
            match ch {
                '"' => return Ok(out),
                '\\' => {
                    let Some(escape) = self.next() else {
                        return Err("unterminated JSON escape".into());
                    };
                    match escape {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{08}'),
                        'f' => out.push('\u{0c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => out.push(self.unicode_escape()?),
                        _ => return Err(format!("invalid JSON escape '\\{escape}'")),
                    }
                }
                ch if ch < ' ' => return Err("unescaped control character in JSON string".into()),
                ch => out.push(ch),
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let first = self.hex4()?;
        if (0xd800..0xdc00).contains(&first) {
            if self.next() != Some('\\') || self.next() != Some('u') {
                return Err("invalid JSON surrogate pair".into());
            }
            let second = self.hex4()?;
            if !(0xdc00..0xe000).contains(&second) {
                return Err("invalid JSON surrogate pair".into());
            }
            let code = 0x10000 + (((first - 0xd800) as u32) << 10) + (second - 0xdc00) as u32;
            return char::from_u32(code).ok_or_else(|| "invalid Unicode code point".to_string());
        }
        if (0xdc00..0xe000).contains(&first) {
            return Err("unexpected JSON low surrogate".into());
        }
        char::from_u32(first as u32).ok_or_else(|| "invalid Unicode code point".to_string())
    }

    fn hex4(&mut self) -> Result<u16, String> {
        let mut value = 0u16;
        for _ in 0..4 {
            let Some(ch) = self.next() else {
                return Err("unterminated JSON Unicode escape".into());
            };
            let Some(digit) = ch.to_digit(16) else {
                return Err(format!("invalid hex digit '{ch}' in JSON Unicode escape"));
            };
            value = (value << 4) | digit as u16;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        self.consume('-');
        if self.consume('0') {
        } else {
            self.digits()?;
        }
        if self.consume('.') {
            self.digits()?;
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.pos += 1;
            }
            self.digits()?;
        }
        Ok(Json::Number(self.chars[start..self.pos].iter().collect()))
    }

    fn digits(&mut self) -> Result<(), String> {
        let start = self.pos;
        while matches!(self.peek(), Some('0'..='9')) {
            self.pos += 1;
        }
        if start == self.pos {
            return Err(format!("expected digit at character {}", self.pos));
        }
        Ok(())
    }

    fn literal(&mut self, text: &str, value: Json) -> Result<Json, String> {
        for expected in text.chars() {
            if self.next() != Some(expected) {
                return Err(format!("invalid JSON literal at character {}", self.pos));
            }
        }
        Ok(value)
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(' ' | '\n' | '\r' | '\t')) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        Some(ch)
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        if self.consume(expected) {
            Ok(())
        } else {
            Err(format!("expected '{expected}' at character {}", self.pos))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Json;

    #[test]
    fn round_trip() {
        let value = Json::object(vec![
            ("text", Json::from("a \"quoted\" word\n")),
            ("count", Json::int(-42)),
            ("ok", Json::Bool(true)),
            (
                "items",
                Json::Array(vec![Json::Null, Json::from("é"), Json::uint(7)]),
            ),
        ]);
        assert_eq!(Json::parse(&value.encode()).unwrap(), value);
    }

    #[test]
    fn parses_escapes_and_numbers() {
        let value = Json::parse(r#"{"s":"\ud800\udc00","n":-12.5e2}"#).unwrap();
        assert_eq!(value.get("s").unwrap().as_str(), Some("𐀀"));
        assert_eq!(value.get("n").unwrap().as_i64(), Some(-1250));
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(Json::parse("[1,]").is_err());
        assert!(Json::parse("{\"a\":}").is_err());
        assert!(Json::parse("01").is_err());
    }
}
