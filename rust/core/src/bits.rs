//! Low-level operations on little-endian multi-word bit arrays
//! (`words[0]` holds bits 0..63), used by the generic model/solver code
//! for boards that need more than one word. Allocation-free; destination
//! slices may alias sources where meaningful (same contract as Java).

#[inline]
pub fn test(a: &[u64], index: i32) -> bool {
    let i = index as usize;
    (a[i >> 6] >> (i & 63)) & 1 != 0
}

#[inline]
pub fn set(a: &mut [u64], index: i32) {
    let i = index as usize;
    a[i >> 6] |= 1u64 << (i & 63);
}

#[inline]
pub fn clear(a: &mut [u64], index: i32) {
    let i = index as usize;
    a[i >> 6] &= !(1u64 << (i & 63));
}

/// `a &= b`
#[inline]
pub fn and_assign(a: &mut [u64], b: &[u64]) {
    for i in 0..a.len() {
        a[i] &= b[i];
    }
}

/// `a |= b`
#[inline]
pub fn or_assign(a: &mut [u64], b: &[u64]) {
    for i in 0..a.len() {
        a[i] |= b[i];
    }
}

#[inline]
pub fn any(a: &[u64]) -> bool {
    a.iter().any(|&w| w != 0)
}

#[inline]
pub fn bit_count(a: &[u64]) -> u32 {
    a.iter().map(|w| w.count_ones()).sum()
}

#[inline]
pub fn and_count(a: &[u64], b: &[u64]) -> u32 {
    a.iter().zip(b).map(|(x, y)| (x & y).count_ones()).sum()
}

#[inline]
pub fn xor_into(a: &[u64], b: &[u64], dest: &mut [u64]) {
    for i in 0..dest.len() {
        dest[i] = a[i] ^ b[i];
    }
}

#[inline]
pub fn and_into(a: &[u64], b: &[u64], dest: &mut [u64]) {
    for i in 0..dest.len() {
        dest[i] = a[i] & b[i];
    }
}

#[inline]
pub fn or_into(a: &[u64], b: &[u64], dest: &mut [u64]) {
    for i in 0..dest.len() {
        dest[i] = a[i] | b[i];
    }
}

/// `dest = a & !b`
#[inline]
pub fn and_not_into(a: &[u64], b: &[u64], dest: &mut [u64]) {
    for i in 0..dest.len() {
        dest[i] = a[i] & !b[i];
    }
}

/// Logical shift towards lower indices: `dest = src >> shift`. Alias-safe.
pub fn shift_right(src: &[u64], shift: u32, dest: &mut [u64]) {
    let n = dest.len();
    let word_shift = (shift >> 6) as usize;
    let bit_shift = shift & 63;
    // Copy through a small stack buffer when src may alias dest is the
    // caller's problem in Rust (borrow checker forbids aliasing &mut),
    // so this is a straight loop.
    for i in 0..n {
        let mut v = 0u64;
        let s = i + word_shift;
        if s < src.len() {
            v = src[s] >> bit_shift;
            if bit_shift != 0 && s + 1 < src.len() {
                v |= src[s + 1] << (64 - bit_shift);
            }
        }
        dest[i] = v;
    }
}

/// Shift towards higher indices: `dest = src << shift`.
pub fn shift_left(src: &[u64], shift: u32, dest: &mut [u64]) {
    let n = dest.len();
    let word_shift = (shift >> 6) as usize;
    let bit_shift = shift & 63;
    for i in (0..n).rev() {
        let mut v = 0u64;
        if i >= word_shift {
            let s = i - word_shift;
            if s < src.len() {
                v = src[s] << bit_shift;
                if bit_shift != 0 && s >= 1 {
                    v |= src[s - 1] >> (64 - bit_shift);
                }
            }
        }
        dest[i] = v;
    }
}

/// `amount > 0` shifts left, `amount < 0` shifts right.
pub fn signed_shift_words(src: &[u64], amount: i32, dest: &mut [u64]) {
    if amount == 0 {
        dest.copy_from_slice(src);
    } else if amount > 0 {
        shift_left(src, amount as u32, dest);
    } else {
        shift_right(src, (-amount) as u32, dest);
    }
}

#[inline]
pub fn signed_shift(value: u64, amount: i32) -> u64 {
    if amount == 0 {
        value
    } else if amount >= 64 || amount <= -64 {
        0
    } else if amount > 0 {
        value << amount
    } else {
        value >> (-amount)
    }
}

/// Unsigned lexicographic comparison from the most significant word down.
pub fn compare_unsigned(a: &[u64], b: &[u64]) -> core::cmp::Ordering {
    for i in (0..a.len()).rev() {
        match a[i].cmp(&b[i]) {
            core::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    core::cmp::Ordering::Equal
}
