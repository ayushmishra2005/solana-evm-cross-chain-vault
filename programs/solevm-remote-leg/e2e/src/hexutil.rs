//! Hex helpers for the FFI boundary.

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2).saturating_add(2));
    out.push_str("0x");
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn encode_array(bytes: &[u8; 32]) -> String {
    encode(bytes)
}

pub fn decode(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim();
    let text = text.strip_prefix("0x").unwrap_or(text);
    if !text.len().is_multiple_of(2) {
        return Err(String::from("hex needs an even length"));
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index + 1 < chars.len() {
        let pair = format!("{}{}", chars[index], chars[index + 1]);
        let byte = u8::from_str_radix(&pair, 16).map_err(|_| format!("bad hex pair {pair}"))?;
        out.push(byte);
        index += 2;
    }
    Ok(out)
}

pub fn decode32(text: &str) -> Result<[u8; 32], String> {
    let bytes = decode(text)?;
    bytes
        .try_into()
        .map_err(|_| String::from("expected 32 bytes"))
}
