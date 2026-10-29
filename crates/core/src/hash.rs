//! Semantic per-subtree hash (xxh3). Equal JSON values hash equal:
//! - object key order is irrelevant; array element order is not;
//! - strings are compared decoded (`"\u0041"` == `"A"`);
//! - numbers are compared by exact decimal value (`1` == `1.0` == `10e-1`);
//! - the type is part of the hash (`"1"` != `1`).

use xxhash_rust::xxh3::xxh3_64_with_seed;

const TAG_NULL: u64 = 0x6a09_e667_f3bc_c908;
const TAG_FALSE: u64 = 0xbb67_ae85_84ca_a73b;
const TAG_TRUE: u64 = 0x3c6e_f372_fe94_f82b;
const TAG_NUMBER: u64 = 0xa54f_f53a_5f1d_36f1;
const TAG_STRING: u64 = 0x510e_527f_ade6_82d1;
const TAG_ARRAY: u64 = 0x9b05_688c_2b3e_6c1f;
const TAG_OBJECT: u64 = 0x1f83_d9ab_fb41_bd6b;
const TAG_MEMBER: u64 = 0x5be0_cd19_137e_2179;

pub(crate) fn null() -> u64 {
    xxh3_64_with_seed(&[], TAG_NULL)
}

pub(crate) fn boolean(value: bool) -> u64 {
    xxh3_64_with_seed(&[], if value { TAG_TRUE } else { TAG_FALSE })
}

pub(crate) fn string(decoded: &[u8]) -> u64 {
    xxh3_64_with_seed(decoded, TAG_STRING)
}

/// `scratch` is a reusable buffer.
pub(crate) fn number(raw: &[u8], scratch: &mut Vec<u8>) -> u64 {
    canonical_number(raw, scratch);
    xxh3_64_with_seed(scratch, TAG_NUMBER)
}

pub(crate) fn array_start() -> u64 {
    TAG_ARRAY
}

/// Order-dependent.
pub(crate) fn array_push(acc: u64, child: u64) -> u64 {
    xxh3_64_with_seed(&child.to_le_bytes(), acc)
}

pub(crate) fn array_finish(acc: u64, count: u32) -> u64 {
    xxh3_64_with_seed(&count.to_le_bytes(), acc)
}

/// Members are summed by the caller, so their order does not matter.
pub(crate) fn member(decoded_key: &[u8], child: u64) -> u64 {
    xxh3_64_with_seed(decoded_key, child ^ TAG_MEMBER)
}

pub(crate) fn object_finish(members_sum: u64, count: u32) -> u64 {
    let mut bytes = [0u8; 12];
    bytes[..8].copy_from_slice(&members_sum.to_le_bytes());
    bytes[8..].copy_from_slice(&count.to_le_bytes());
    xxh3_64_with_seed(&bytes, TAG_OBJECT)
}

/// Writes the canonical form of a valid JSON number literal:
/// value = ±0.d1d2…dn × 10^point, without leading or trailing zeros.
/// Layout: `[sign][point: i64 LE][digits]`; zero is always `[0]` (`-0` == `0`).
/// Huge exponents saturate to the `i64` limits.
pub(crate) fn canonical_number(raw: &[u8], out: &mut Vec<u8>) {
    out.clear();
    let (negative, rest) = match raw.split_first() {
        Some((b'-', rest)) => (true, rest),
        _ => (false, raw),
    };
    let mantissa_end = rest
        .iter()
        .position(|&c| c == b'e' || c == b'E')
        .unwrap_or(rest.len());
    let (mantissa, exponent_part) = rest.split_at(mantissa_end);
    let exponent = parse_exponent(exponent_part.get(1..).unwrap_or_default());
    let (int, frac) = match mantissa.iter().position(|&c| c == b'.') {
        Some(dot) => (
            &mantissa[..dot],
            mantissa.get(dot + 1..).unwrap_or_default(),
        ),
        None => (mantissa, &[][..]),
    };

    let total = int.len() + frac.len();
    let digits = || int.iter().chain(frac);
    let leading = digits().take_while(|&&c| c == b'0').count();
    if leading == total {
        out.push(0);
        return;
    }
    let trailing = digits().rev().take_while(|&&c| c == b'0').count();
    let point = exponent
        .saturating_add(int.len() as i64)
        .saturating_sub(leading as i64);
    out.push(if negative { 2 } else { 1 });
    out.extend_from_slice(&point.to_le_bytes());
    out.extend(digits().skip(leading).take(total - leading - trailing));
}

fn parse_exponent(bytes: &[u8]) -> i64 {
    let (negative, digits) = match bytes.split_first() {
        Some((b'-', rest)) => (true, rest),
        Some((b'+', rest)) => (false, rest),
        _ => (false, bytes),
    };
    let magnitude = digits.iter().fold(0i64, |acc, &c| {
        acc.saturating_mul(10).saturating_add(i64::from(c - b'0'))
    });
    if negative { -magnitude } else { magnitude }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canon(raw: &str) -> Vec<u8> {
        let mut out = Vec::new();
        canonical_number(raw.as_bytes(), &mut out);
        out
    }

    #[test]
    fn equal_numbers_share_canonical_form() {
        let groups: &[&[&str]] = &[
            &[
                "1", "1.0", "1.000", "10e-1", "0.1e1", "100E-2", "1e0", "1E+0",
            ],
            &["0", "-0", "0.0", "-0.0e10", "0e-5"],
            &["-12.5", "-125e-1", "-1.25E1", "-0.0125e3"],
            &["1500", "1.5e3", "15e2", "1500.00"],
        ];
        for group in groups {
            for raw in group.iter() {
                assert_eq!(canon(raw), canon(group[0]), "{raw} == {}", group[0]);
            }
        }
    }

    #[test]
    fn different_numbers_differ() {
        let distinct = [
            "1",
            "-1",
            "2",
            "10",
            "0.1",
            "11",
            "1.01",
            "0",
            "1e400",
            "1e401",
            "12345678901234567890",
            "12345678901234567891",
        ];
        for (i, a) in distinct.iter().enumerate() {
            for b in &distinct[i + 1..] {
                assert_ne!(canon(a), canon(b), "{a} != {b}");
            }
        }
    }

    #[test]
    fn huge_exponents_saturate_without_panicking() {
        assert_eq!(
            canon("1e99999999999999999999"),
            canon("1e99999999999999999998")
        );
        assert_ne!(
            canon("1e-99999999999999999999"),
            canon("1e99999999999999999999")
        );
    }

    #[test]
    fn container_tags_differ() {
        assert_ne!(array_finish(array_start(), 0), object_finish(0, 0));
        assert_ne!(null(), boolean(false));
        assert_ne!(string(b""), null());
    }
}
