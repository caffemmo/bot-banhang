const TOTP_PERIOD_SECONDS: u64 = 30;
const TOTP_DIGITS: u32 = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TotpCode {
    pub code: String,
    pub seconds_remaining: u8,
}

pub fn current_totp_code(secret: &str, now_unix: u64) -> Option<TotpCode> {
    let key = decode_base32_secret(secret)?;
    let counter = now_unix / TOTP_PERIOD_SECONDS;
    let code = hotp(&key, counter, TOTP_DIGITS)?;
    let seconds_remaining = (TOTP_PERIOD_SECONDS - (now_unix % TOTP_PERIOD_SECONDS)) as u8;

    Some(TotpCode {
        code,
        seconds_remaining,
    })
}

fn hotp(key: &[u8], counter: u64, digits: u32) -> Option<String> {
    if key.is_empty() || digits == 0 || digits > 9 {
        return None;
    }

    let digest = hmac_sha1(key, &counter.to_be_bytes());
    let offset = (digest[19] & 0x0f) as usize;
    let binary = (((digest[offset] & 0x7f) as u32) << 24)
        | ((digest[offset + 1] as u32) << 16)
        | ((digest[offset + 2] as u32) << 8)
        | (digest[offset + 3] as u32);
    let code = binary % 10u32.pow(digits);

    Some(format!("{:0width$}", code, width = digits as usize))
}

fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    const BLOCK_SIZE: usize = 64;
    let mut normalized_key = [0_u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        normalized_key[..20].copy_from_slice(&sha1_digest(key));
    } else {
        normalized_key[..key.len()].copy_from_slice(key);
    }

    let mut inner = Vec::with_capacity(BLOCK_SIZE + message.len());
    let mut outer = Vec::with_capacity(BLOCK_SIZE + 20);
    for byte in normalized_key {
        inner.push(byte ^ 0x36);
        outer.push(byte ^ 0x5c);
    }
    inner.extend_from_slice(message);
    outer.extend_from_slice(&sha1_digest(&inner));
    sha1_digest(&outer)
}

fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let bit_len = (data.len() as u64) * 8;
    let mut padded = data.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    let mut h0 = 0x67452301_u32;
    let mut h1 = 0xefcdab89_u32;
    let mut h2 = 0x98badcfe_u32;
    let mut h3 = 0x10325476_u32;
    let mut h4 = 0xc3d2e1f0_u32;

    for chunk in padded.chunks_exact(64) {
        let mut words = [0_u32; 80];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes([
                chunk[offset],
                chunk[offset + 1],
                chunk[offset + 2],
                chunk[offset + 3],
            ]);
        }
        for index in 16..80 {
            words[index] = (words[index - 3]
                ^ words[index - 8]
                ^ words[index - 14]
                ^ words[index - 16])
                .rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;
        for (index, word) in words.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut digest = [0_u8; 20];
    for (index, value) in [h0, h1, h2, h3, h4].iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
    }
    digest
}

fn decode_base32_secret(secret: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u8;

    for ch in secret.chars().filter(|ch| !ch.is_ascii_whitespace()) {
        if ch == '=' {
            continue;
        }
        let value = match ch.to_ascii_uppercase() {
            'A'..='Z' => ch.to_ascii_uppercase() as u32 - 'A' as u32,
            '2'..='7' => ch as u32 - '2' as u32 + 26,
            _ => return None,
        };
        buffer = (buffer << 5) | value;
        bits += 5;

        while bits >= 8 {
            bits -= 8;
            bytes.push((buffer >> bits) as u8);
            buffer = if bits == 0 {
                0
            } else {
                buffer & ((1u32 << bits) - 1)
            };
        }
    }

    if bytes.is_empty() { None } else { Some(bytes) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_matches_rfc6238_sha1_vector_truncated_to_six_digits() {
        let code = current_totp_code("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ", 59).unwrap();

        assert_eq!(code.code, "287082");
        assert_eq!(code.seconds_remaining, 1);
    }

    #[test]
    fn totp_accepts_lowercase_and_spaces() {
        let code = current_totp_code("gez dgnbvgy3tqojq gezdgnbvgy3tqojq", 59).unwrap();

        assert_eq!(code.code, "287082");
    }

    #[test]
    fn totp_rejects_invalid_base32_secret() {
        assert!(current_totp_code("not-a-secret!", 59).is_none());
    }
}
