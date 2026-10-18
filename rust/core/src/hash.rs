//! A small fast hasher (FxHash-style multiply-xor) for transposition-table
//! `HashMap`s. The default SipHash is needlessly expensive for u64-slice
//! keys we generate ourselves; this keeps the crate dependency-free.

use core::hash::{BuildHasherDefault, Hasher};
use std::collections::HashMap;

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

#[derive(Default)]
pub struct FastHasher {
    hash: u64,
}

impl FastHasher {
    #[inline]
    fn add(&mut self, value: u64) {
        self.hash = (self.hash.rotate_left(5) ^ value).wrapping_mul(SEED);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rem) = bytes.as_chunks::<8>();
        for c in chunks {
            self.add(u64::from_le_bytes(*c));
        }
        if !rem.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rem.len()].copy_from_slice(rem);
            self.add(u64::from_le_bytes(buf));
        }
    }
    #[inline]
    fn write_u8(&mut self, n: u8) {
        self.add(n as u64);
    }
    #[inline]
    fn write_u32(&mut self, n: u32) {
        self.add(n as u64);
    }
    #[inline]
    fn write_u64(&mut self, n: u64) {
        self.add(n);
    }
    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }
    #[inline]
    fn write_i32(&mut self, n: i32) {
        self.add(n as u64);
    }
    #[inline]
    fn write_i64(&mut self, n: i64) {
        self.add(n as u64);
    }
}

pub type BuildFast = BuildHasherDefault<FastHasher>;
pub type FastMap<K, V> = HashMap<K, V, BuildFast>;

pub fn fast_map_with_capacity<K, V>(cap: usize) -> FastMap<K, V> {
    HashMap::with_capacity_and_hasher(cap, BuildFast::default())
}

/// SplitMix64 finalizer, used for open-addressing tables keyed by u64 pairs.
#[inline]
pub fn mix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

#[inline]
pub fn mix_pair(a: u64, b: u64) -> u64 {
    mix64(a.wrapping_add(0x9e37_79b9_7f4a_7c15) ^ mix64(b))
}
