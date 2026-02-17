use crate::error::{Error, Result};

pub fn encode_header_block(headers: &[(&str, &str)]) -> Vec<u8> {
    let mut buf = Vec::new();

    // Required Insert Count = 0, Delta Base = 0
    buf.push(0x00);
    buf.push(0x00);

    for &(name, value) in headers {
        encode_literal_with_literal_name(&mut buf, name.as_bytes(), value.as_bytes());
    }

    buf
}

pub fn decode_header_block(data: &[u8]) -> Result<Vec<(String, String)>> {
    if data.len() < 2 {
        return Err(Error::QpackError("header block too short".into()));
    }

    let mut offset = 0;

    // Required Insert Count
    let (_ric, n) = decode_prefix_int(&data[offset..], 8)?;
    offset += n;

    // Delta Base
    let (_, n) = decode_prefix_int(&data[offset..], 7)?;
    offset += n;

    let mut headers = Vec::new();

    while offset < data.len() {
        let first_byte = data[offset];

        if first_byte & 0x80 != 0 {
            // Indexed Field Line (static) — 1 T IIIIII
            let static_bit = first_byte & 0x40 != 0;
            let (index, n) = decode_prefix_int(&data[offset..], 6)?;
            offset += n;

            if static_bit {
                if let Some((name, value)) = static_table_get(index as usize) {
                    headers.push((name.to_string(), value.to_string()));
                } else {
                    return Err(Error::QpackError(format!(
                        "static index {index} out of range"
                    )));
                }
            } else {
                return Err(Error::QpackError("dynamic table not supported".into()));
            }
        } else if first_byte & 0xc0 == 0x40 {
            // Literal with Name Reference — 01 N T III
            let static_bit = first_byte & 0x10 != 0;
            let (index, n) = decode_prefix_int(&data[offset..], 4)?;
            offset += n;

            let name = if static_bit {
                static_table_get(index as usize)
                    .map(|(n, _)| n.to_string())
                    .ok_or_else(|| {
                        Error::QpackError(format!("static index {index} out of range"))
                    })?
            } else {
                return Err(Error::QpackError("dynamic table not supported".into()));
            };

            let (value, n) = decode_string(&data[offset..])?;
            offset += n;
            headers.push((name, value));
        } else if first_byte & 0xe0 == 0x20 {
            // Literal with Literal Name — 001 N HHHH
            let (name, n) = decode_string_with_prefix(&data[offset..], 3)?;
            offset += n;
            let (value, n) = decode_string(&data[offset..])?;
            offset += n;
            headers.push((name, value));
        } else {
            return Err(Error::QpackError(format!(
                "unsupported field line encoding: {first_byte:#04x}"
            )));
        }
    }

    Ok(headers)
}

fn encode_literal_with_literal_name(buf: &mut Vec<u8>, name: &[u8], value: &[u8]) {
    // First byte: 001 (literal) | N=0 (not sensitive) | H=0 (no Huffman for name)
    encode_prefix_int_into(buf, name.len() as u64, 3, 0x20);
    buf.extend_from_slice(name);

    // Value: H=0 (no Huffman)
    encode_prefix_int_into(buf, value.len() as u64, 7, 0x00);
    buf.extend_from_slice(value);
}

fn encode_prefix_int_into(buf: &mut Vec<u8>, value: u64, prefix_bits: u8, first_byte_mask: u8) {
    let max_prefix = (1u64 << prefix_bits) - 1;

    if value < max_prefix {
        buf.push(first_byte_mask | (value as u8));
    } else {
        buf.push(first_byte_mask | max_prefix as u8);
        let mut remaining = value - max_prefix;
        while remaining >= 128 {
            buf.push(0x80 | (remaining & 0x7f) as u8);
            remaining >>= 7;
        }
        buf.push(remaining as u8);
    }
}

fn decode_prefix_int(data: &[u8], prefix_bits: u8) -> Result<(u64, usize)> {
    if data.is_empty() {
        return Err(Error::QpackError("unexpected end of prefix int".into()));
    }

    let max_prefix = (1u64 << prefix_bits) - 1;
    let mut value = (data[0] as u64) & max_prefix;
    let mut offset = 1;

    if value < max_prefix {
        return Ok((value, offset));
    }

    let mut shift = 0u32;
    loop {
        if offset >= data.len() {
            return Err(Error::QpackError("truncated prefix int".into()));
        }
        let byte = data[offset] as u64;
        offset += 1;
        value += (byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, offset));
        }
        shift += 7;
        if shift > 63 {
            return Err(Error::QpackError("prefix int overflow".into()));
        }
    }
}

fn decode_string(data: &[u8]) -> Result<(String, usize)> {
    decode_string_with_prefix(data, 7)
}

fn decode_string_with_prefix(data: &[u8], prefix_bits: u8) -> Result<(String, usize)> {
    if data.is_empty() {
        return Err(Error::QpackError("unexpected end of string".into()));
    }

    let huffman = data[0] & (1 << prefix_bits) != 0;
    let (length, n) = decode_prefix_int(data, prefix_bits)?;
    let length = length as usize;

    if data.len() < n + length {
        return Err(Error::QpackError("truncated string".into()));
    }

    let raw = &data[n..n + length];

    let s = if huffman {
        let decoded = huffman_decode(raw)?;
        String::from_utf8(decoded)
            .map_err(|_| Error::QpackError("invalid UTF-8 in Huffman string".into()))?
    } else {
        String::from_utf8(raw.to_vec())
            .map_err(|_| Error::QpackError("invalid UTF-8 in string".into()))?
    };

    Ok((s, n + length))
}

// RFC 7541 Appendix B — HPACK Huffman table (same used by QPACK)
// Each entry: (code_bits, bit_length)
static HUFFMAN_ENCODE: [(u32, u8); 257] = [
    (0x1ff8, 13),
    (0x7fffd8, 23),
    (0xfffffe2, 28),
    (0xfffffe3, 28),
    (0xfffffe4, 28),
    (0xfffffe5, 28),
    (0xfffffe6, 28),
    (0xfffffe7, 28),
    (0xfffffe8, 28),
    (0xffffea, 24),
    (0x3ffffffc, 30),
    (0xfffffe9, 28),
    (0xfffffea, 28),
    (0x3ffffffd, 30),
    (0xfffffeb, 28),
    (0xfffffec, 28),
    (0xfffffed, 28),
    (0xfffffee, 28),
    (0xfffffef, 28),
    (0xffffff0, 28),
    (0xffffff1, 28),
    (0xffffff2, 28),
    (0x3ffffffe, 30),
    (0xffffff3, 28),
    (0xffffff4, 28),
    (0xffffff5, 28),
    (0xffffff6, 28),
    (0xffffff7, 28),
    (0xffffff8, 28),
    (0xffffff9, 28),
    (0xffffffa, 28),
    (0xffffffb, 28),
    (0x14, 6),
    (0x3f8, 10),
    (0x3f9, 10),
    (0xffa, 12),
    (0x1ff9, 13),
    (0x15, 6),
    (0xf8, 8),
    (0x7fa, 11),
    (0x3fa, 10),
    (0x3fb, 10),
    (0xf9, 8),
    (0x7fb, 11),
    (0xfa, 8),
    (0x16, 6),
    (0x17, 6),
    (0x18, 6),
    (0x0, 5),
    (0x1, 5),
    (0x2, 5),
    (0x19, 6),
    (0x1a, 6),
    (0x1b, 6),
    (0x1c, 6),
    (0x1d, 6),
    (0x1e, 6),
    (0x1f, 6),
    (0x5c, 7),
    (0xfb, 8),
    (0x7ffc, 15),
    (0x20, 6),
    (0xffb, 12),
    (0x3fc, 10),
    (0x1ffa, 13),
    (0x21, 6),
    (0x5d, 7),
    (0x5e, 7),
    (0x5f, 7),
    (0x60, 7),
    (0x61, 7),
    (0x62, 7),
    (0x63, 7),
    (0x64, 7),
    (0x65, 7),
    (0x66, 7),
    (0x67, 7),
    (0x68, 7),
    (0x69, 7),
    (0x6a, 7),
    (0x6b, 7),
    (0x6c, 7),
    (0x6d, 7),
    (0x6e, 7),
    (0x6f, 7),
    (0x70, 7),
    (0x71, 7),
    (0x72, 7),
    (0xfc, 8),
    (0x73, 7),
    (0xfd, 8),
    (0x1ffb, 13),
    (0x7fff0, 19),
    (0x1ffc, 13),
    (0x3ffc, 14),
    (0x22, 6),
    (0x7ffd, 15),
    (0x3, 5),
    (0x23, 6),
    (0x4, 5),
    (0x24, 6),
    (0x5, 5),
    (0x25, 6),
    (0x26, 6),
    (0x27, 6),
    (0x6, 5),
    (0x74, 7),
    (0x75, 7),
    (0x28, 6),
    (0x29, 6),
    (0x2a, 6),
    (0x7, 5),
    (0x2b, 6),
    (0x76, 7),
    (0x2c, 6),
    (0x8, 5),
    (0x9, 5),
    (0x2d, 6),
    (0x77, 7),
    (0x78, 7),
    (0x79, 7),
    (0x7a, 7),
    (0x7b, 7),
    (0x7ffe, 15),
    (0x7fc, 11),
    (0x3ffd, 14),
    (0x1ffd, 13),
    (0xffffffc, 28),
    (0xfffe6, 20),
    (0x3fffd2, 22),
    (0xfffe7, 20),
    (0xfffe8, 20),
    (0x3fffd3, 22),
    (0x3fffd4, 22),
    (0x3fffd5, 22),
    (0x7fffd9, 23),
    (0x3fffd6, 22),
    (0x7fffda, 23),
    (0x7fffdb, 23),
    (0x7fffdc, 23),
    (0x7fffdd, 23),
    (0x7fffde, 23),
    (0xffffeb, 24),
    (0x7fffdf, 23),
    (0xffffec, 24),
    (0xffffed, 24),
    (0x3fffd7, 22),
    (0x7fffe0, 23),
    (0xffffee, 24),
    (0x7fffe1, 23),
    (0x7fffe2, 23),
    (0x7fffe3, 23),
    (0x7fffe4, 23),
    (0x1fffdc, 21),
    (0x3fffd8, 22),
    (0x7fffe5, 23),
    (0x3fffd9, 22),
    (0x7fffe6, 23),
    (0x7fffe7, 23),
    (0xffffef, 24),
    (0x3fffda, 22),
    (0x1fffdd, 21),
    (0xfffe9, 20),
    (0x3fffdb, 22),
    (0x3fffdc, 22),
    (0x7fffe8, 23),
    (0x7fffe9, 23),
    (0x1fffde, 21),
    (0x7fffea, 23),
    (0x3fffdd, 22),
    (0x3fffde, 22),
    (0xfffff0, 24),
    (0x1fffdf, 21),
    (0x3fffdf, 22),
    (0x7fffeb, 23),
    (0x7fffec, 23),
    (0x1fffe0, 21),
    (0x1fffe1, 21),
    (0x3fffe0, 22),
    (0x1fffe2, 21),
    (0x7fffed, 23),
    (0x3fffe1, 22),
    (0x7fffee, 23),
    (0x7fffef, 23),
    (0xfffea, 20),
    (0x3fffe2, 22),
    (0x3fffe3, 22),
    (0x3fffe4, 22),
    (0x7ffff0, 23),
    (0x3fffe5, 22),
    (0x3fffe6, 22),
    (0x7ffff1, 23),
    (0x3ffffe0, 26),
    (0x3ffffe1, 26),
    (0xfffeb, 20),
    (0x7fff1, 19),
    (0x3fffe7, 22),
    (0x7ffff2, 23),
    (0x3fffe8, 22),
    (0x1ffffec, 25),
    (0x3ffffe2, 26),
    (0x3ffffe3, 26),
    (0x3ffffe4, 26),
    (0x7ffffde, 27),
    (0x7ffffdf, 27),
    (0x3ffffe5, 26),
    (0xfffff1, 24),
    (0x1ffffed, 25),
    (0x7fff2, 19),
    (0x1fffe3, 21),
    (0x3ffffe6, 26),
    (0x7ffffe0, 27),
    (0x7ffffe1, 27),
    (0x3ffffe7, 26),
    (0x7ffffe2, 27),
    (0xfffff2, 24),
    (0x1fffe4, 21),
    (0x1fffe5, 21),
    (0x3ffffe8, 26),
    (0x3ffffe9, 26),
    (0xffffffd, 28),
    (0x7ffffe3, 27),
    (0x7ffffe4, 27),
    (0x7ffffe5, 27),
    (0xfffec, 20),
    (0xfffff3, 24),
    (0xfffed, 20),
    (0x1fffe6, 21),
    (0x3fffe9, 22),
    (0x1fffe7, 21),
    (0x1fffe8, 21),
    (0x7ffff3, 23),
    (0x3fffea, 22),
    (0x3fffeb, 22),
    (0x1ffffee, 25),
    (0x1ffffef, 25),
    (0xfffff4, 24),
    (0xfffff5, 24),
    (0x3ffffea, 26),
    (0x7ffff4, 23),
    (0x3ffffeb, 26),
    (0x7ffffe6, 27),
    (0x3ffffec, 26),
    (0x3ffffed, 26),
    (0x7ffffe7, 27),
    (0x7ffffe8, 27),
    (0x7ffffe9, 27),
    (0x7ffffea, 27),
    (0x7ffffeb, 27),
    (0xffffffe, 28),
    (0x7ffffec, 27),
    (0x7ffffed, 27),
    (0x7ffffee, 27),
    (0x7ffffef, 27),
    (0x7fffff0, 27),
    (0x3ffffee, 26),
    (0x3fffffff, 30),
];

static STATIC_TABLE: [(&str, &str); 99] = [
    (":authority", ""),
    (":path", "/"),
    ("age", "0"),
    ("content-disposition", ""),
    ("content-length", "0"),
    ("cookie", ""),
    ("date", ""),
    ("etag", ""),
    ("if-modified-since", ""),
    ("if-none-match", ""),
    ("last-modified", ""),
    ("link", ""),
    ("location", ""),
    ("referer", ""),
    ("set-cookie", ""),
    (":method", "CONNECT"),
    (":method", "DELETE"),
    (":method", "GET"),
    (":method", "HEAD"),
    (":method", "OPTIONS"),
    (":method", "POST"),
    (":method", "PUT"),
    (":scheme", "http"),
    (":scheme", "https"),
    (":status", "103"),
    (":status", "200"),
    (":status", "304"),
    (":status", "404"),
    (":status", "503"),
    ("accept", "*/*"),
    ("accept", "application/dns-message"),
    ("accept-encoding", "gzip, deflate, br"),
    ("accept-ranges", "bytes"),
    ("access-control-allow-headers", "cache-control"),
    ("access-control-allow-headers", "content-type"),
    ("access-control-allow-origin", "*"),
    ("cache-control", "max-age=0"),
    ("cache-control", "max-age=2592000"),
    ("cache-control", "max-age=604800"),
    ("cache-control", "no-cache"),
    ("cache-control", "no-store"),
    ("cache-control", "public, max-age=31536000"),
    ("content-encoding", "br"),
    ("content-encoding", "gzip"),
    ("content-type", "application/dns-message"),
    ("content-type", "application/javascript"),
    ("content-type", "application/json"),
    ("content-type", "application/x-www-form-urlencoded"),
    ("content-type", "image/gif"),
    ("content-type", "image/jpeg"),
    ("content-type", "image/png"),
    ("content-type", "text/css"),
    ("content-type", "text/html; charset=utf-8"),
    ("content-type", "text/plain"),
    ("content-type", "text/plain;charset=utf-8"),
    ("range", "bytes=0-"),
    ("strict-transport-security", "max-age=31536000"),
    (
        "strict-transport-security",
        "max-age=31536000; includesubdomains",
    ),
    (
        "strict-transport-security",
        "max-age=31536000; includesubdomains; preload",
    ),
    ("vary", "accept-encoding"),
    ("vary", "origin"),
    ("x-content-type-options", "nosniff"),
    ("x-xss-protection", "1; mode=block"),
    (":status", "100"),
    (":status", "204"),
    (":status", "206"),
    (":status", "302"),
    (":status", "400"),
    (":status", "403"),
    (":status", "421"),
    (":status", "425"),
    (":status", "500"),
    ("accept-language", ""),
    ("access-control-allow-credentials", "FALSE"),
    ("access-control-allow-credentials", "TRUE"),
    ("access-control-allow-headers", "*"),
    ("access-control-allow-methods", "get"),
    ("access-control-allow-methods", "get, post, options"),
    ("access-control-allow-methods", "options"),
    ("access-control-expose-headers", "content-length"),
    ("access-control-request-headers", "content-type"),
    ("access-control-request-method", "get"),
    ("access-control-request-method", "post"),
    ("alt-svc", "clear"),
    ("authorization", ""),
    (
        "content-security-policy",
        "script-src 'none'; object-src 'none'; base-uri 'none'",
    ),
    ("early-data", "1"),
    ("expect-ct", ""),
    ("forwarded", ""),
    ("if-range", ""),
    ("origin", ""),
    ("purpose", "prefetch"),
    ("server", ""),
    ("timing-allow-origin", "*"),
    ("upgrade-insecure-requests", "1"),
    ("user-agent", ""),
    ("x-forwarded-for", ""),
    ("x-frame-options", "deny"),
    ("x-frame-options", "sameorigin"),
];

fn static_table_get(index: usize) -> Option<(&'static str, &'static str)> {
    STATIC_TABLE.get(index).copied()
}

fn huffman_decode(data: &[u8]) -> Result<Vec<u8>> {
    let tree = build_huffman_tree();

    let mut result = Vec::new();
    let mut node_idx: usize = 0;
    for &byte in data.iter() {
        for bit_pos in (0..8).rev() {
            let bit = ((byte >> bit_pos) & 1) as usize;
            let next = tree[node_idx].children[bit];
            if next < 0 {
                return Err(Error::QpackError("invalid Huffman code".into()));
            }
            node_idx = next as usize;
            if tree[node_idx].symbol >= 0 {
                result.push(tree[node_idx].symbol as u8);
                node_idx = 0;
            }
        }
    }

    // Remaining bits must be padding (all 1s, part of EOS)
    Ok(result)
}

struct HNode {
    children: [i32; 2],
    symbol: i16,
}

fn build_huffman_tree() -> Vec<HNode> {
    let mut nodes = vec![HNode {
        children: [-1, -1],
        symbol: -1,
    }];

    for (symbol, &(code, length)) in HUFFMAN_ENCODE[..256].iter().enumerate() {
        let mut idx = 0usize;
        for bit_pos in (0..length).rev() {
            let bit = ((code >> bit_pos) & 1) as usize;
            if nodes[idx].children[bit] < 0 {
                let new_idx = nodes.len();
                nodes.push(HNode {
                    children: [-1, -1],
                    symbol: -1,
                });
                nodes[idx].children[bit] = new_idx as i32;
                idx = new_idx;
            } else {
                idx = nodes[idx].children[bit] as usize;
            }
        }
        nodes[idx].symbol = symbol as i16;
    }

    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let headers = vec![
            (":method", "CONNECT"),
            (":protocol", "webtransport"),
            (":scheme", "https"),
            (":authority", "example.com"),
            (":path", "/wt"),
        ];

        let encoded = encode_header_block(&headers);
        let decoded = decode_header_block(&encoded).unwrap();

        assert_eq!(decoded.len(), headers.len());
        for (i, (name, value)) in decoded.iter().enumerate() {
            assert_eq!(name, headers[i].0);
            assert_eq!(value, headers[i].1);
        }
    }

    #[test]
    fn prefix_int_roundtrip() {
        let mut buf = Vec::new();
        for &val in &[0u64, 1, 30, 31, 127, 128, 1000, 65535] {
            buf.clear();
            encode_prefix_int_into(&mut buf, val, 5, 0x00);
            let (decoded, _) = decode_prefix_int(&buf, 5).unwrap();
            assert_eq!(decoded, val, "roundtrip failed for {val}");
        }
    }

    #[test]
    fn static_table_indexed() {
        // Encode manually: indexed field line for :method=CONNECT (index 15)
        // 1 1 001111 = 0xCF
        let data = vec![0x00, 0x00, 0xcf];
        let headers = decode_header_block(&data).unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, ":method");
        assert_eq!(headers[0].1, "CONNECT");
    }

    #[test]
    fn huffman_decode_basic() {
        // Huffman encode "www.example.com" per RFC 7541 examples
        let huffman_bytes = [
            0xf1, 0xe3, 0xc2, 0xe5, 0xf2, 0x3a, 0x6b, 0xa0, 0xab, 0x90, 0xf4, 0xff,
        ];
        let decoded = huffman_decode(&huffman_bytes).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "www.example.com");
    }
}
